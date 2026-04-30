// to_xml.rs

use crate::common::bpmn_event::BpmnEvent;
use crate::common::graph::Graph;
use crate::common::node::Node;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::Write;

fn xml_id_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, ch) in s.chars().enumerate() {
        let ok = ch.is_ascii_alphanumeric() || ch == '_' || ch == '-';
        if ok {
            if i == 0 && ch.is_ascii_digit() {
                out.push('_');
            }
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}

pub fn generate_bpmn(graph: &Graph) -> String {
    let mut bpmn = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<bpmn:definitions xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
xmlns:bpmn="http://www.omg.org/spec/BPMN/20100524/MODEL"
xmlns:bpmndi="http://www.omg.org/spec/BPMN/20100524/DI"
xmlns:dc="http://www.omg.org/spec/DD/20100524/DC"
xmlns:di="http://www.omg.org/spec/DD/20100524/DI"
xmlns:camunda="http://camunda.org/schema/1.0/bpmn"
xmlns:modeler="http://camunda.org/schema/modeler/1.0" id="Definitions_1"
targetNamespace="http://bpmn.io/schema/bpmn" exporter="Camunda Modeler"
exporterVersion="5.17.0">
"#,
    );

    // Begin collaboration
    bpmn.push_str(r#"  <bpmn:collaboration id="Collaboration_1">"#);

    // Collect unique pool names from nodes
    let pool_names: HashSet<String> = graph.pools.iter().map(|pool| pool.get_pool_name()).collect();

    for pool_name in &pool_names {
        let pool_id = xml_id_safe(pool_name);
        bpmn.push_str(&format!(
            r#"<bpmn:participant id="Participant_{}" name="{}" processRef="Process_{}" />"#,
            pool_id, pool_name, pool_id
        ));
    }

    bpmn.push_str(r#"  </bpmn:collaboration>"#);

    // Generate processes for each pool
    for pool_name in &pool_names {
        let pool_id = xml_id_safe(pool_name);
        bpmn.push_str(&format!(
            r#"<bpmn:process id="Process_{}" isExecutable="true">"#,
            pool_id
        ));

        // Get nodes in this pool
        let pool_nodes: Vec<&Node> = graph.get_nodes_by_pool_name(pool_name);

        // Detect a "loop region" to wrap into a multi-instance subprocess.
        // Heuristic: a task whose name starts with "Loop:" is the region entry,
        // and a task whose name equals "End Loop" is the region exit.
        let loop_region = detect_loop_region(&pool_nodes, graph);

        // Collect unique lane IDs within this pool
        let lane_ids: HashSet<String> = pool_nodes
            .iter()
            .filter_map(|node| node.lane.clone())
            .collect();

        // Generate laneSet if there are lanes
        if !lane_ids.is_empty() {
            bpmn.push_str(&format!(r#"<bpmn:laneSet id="LaneSet_{}">"#, pool_id));

            for lane_id in &lane_ids {
                let lane_safe = xml_id_safe(lane_id);
                bpmn.push_str(&format!(
                    r#"<bpmn:lane id="Lane_{}" name="{}">"#,
                    lane_safe, lane_id
                ));

                // Get nodes in this lane
                let lane_nodes: Vec<&Node> = pool_nodes
                    .iter()
                    .filter(|node| node.lane.as_deref() == Some(lane_id.as_str()))
                    .cloned()
                    .collect();

                // Add flowNodeRefs (keep loop collapsed: show only SubProcess_Loop)
                let mut added_loop_subprocess_ref = false;
                for node in &lane_nodes {
                    if let Some(region) = &loop_region {
                        if region.contains(node.id) {
                            if !added_loop_subprocess_ref {
                                bpmn.push_str(
                                    r#"<bpmn:flowNodeRef>SubProcess_Loop</bpmn:flowNodeRef>"#,
                                );
                                added_loop_subprocess_ref = true;
                            }
                            continue;
                        }
                    }
                    let node_id = get_node_bpmn_id(node);
                    bpmn.push_str(&format!(
                        r#"<bpmn:flowNodeRef>{}</bpmn:flowNodeRef>"#,
                        node_id
                    ));
                }

                bpmn.push_str(r#"</bpmn:lane>"#);
            }

            bpmn.push_str(r#"</bpmn:laneSet>"#);
        }

        // Generate flow nodes (events, tasks, etc.)
        // If a loop region is present, emit an expanded multi-instance subprocess
        // and skip emitting the region nodes at the top level.
        if let Some(region) = &loop_region {
            for node in &pool_nodes {
                if region.contains(node.id) {
                    continue;
                }
                generate_flow_node(&mut bpmn, node, graph);
            }
            generate_multi_instance_subprocess(&mut bpmn, graph, region);
        } else {
            for node in &pool_nodes {
                generate_flow_node(&mut bpmn, node, graph);
            }
        }

        // Generate sequence flows (top-level)
        generate_sequence_flows(&mut bpmn, &graph, &pool_nodes, loop_region.as_ref());

        bpmn.push_str(r#"</bpmn:process>"#);
    }

    // Add BPMN diagram elements
    bpmn.push_str(
        r#"<bpmndi:BPMNDiagram id="BPMNDiagram_1">
  <bpmndi:BPMNPlane id="BPMNPlane_1" bpmnElement="Collaboration_1">
"#,
    );

    // Add BPMN shapes for participants (pools)
    for pool in graph.get_pools() {
        let pool_name = pool.get_pool_name();
        let pool_id = xml_id_safe(&pool_name);
        bpmn.push_str(&format!(
            r#"<bpmndi:BPMNShape id="Participant_{}_di" bpmnElement="Participant_{}" isHorizontal="true">
    <dc:Bounds x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" />
  </bpmndi:BPMNShape>"#,
  pool_id,
  pool_id,
            /* x */ pool.x.unwrap_or(0.0),
            /* y */ pool.y.unwrap_or(0.0),
            /* width */ pool.width.unwrap_or(0.0),
            /* height */ pool.height.unwrap_or(0.0),
        ));

        for lane in pool.get_lanes() {
            let lane_id = lane.get_lane();
            let lane_safe = xml_id_safe(&lane_id);
            bpmn.push_str(&format!(
                r#"<bpmndi:BPMNShape id="Lane_{}_di" bpmnElement="Lane_{}" isHorizontal="true">
    <dc:Bounds x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" />
  </bpmndi:BPMNShape>"#,
                lane_safe,
                lane_safe,
                /* x */ lane.x.unwrap_or(0.0),
                /* y */ lane.y.unwrap_or(0.0),
                /* width */ lane.width.unwrap_or(0.0),
                /* height */ lane.height.unwrap_or(0.0),
            ));
        }
    }

    // Add BPMN shapes for flow nodes
    let loop_layouts = compute_loop_layouts(graph, &pool_names);

    // If a loop subprocess exists, compress TOP-LEVEL columns using only the nodes we actually draw
    // (Start/portfolio/filter/SubProcess/End). This prevents huge empty space caused by hidden
    // internal loop-body nodes that still have large layer_id values.
    let mut top_level_x_override: HashMap<usize, f64> = HashMap::new();
    if !loop_layouts.is_empty() {
        let mut layer_ids: BTreeSet<usize> = BTreeSet::new();
        let mut sample_x_by_layer: HashMap<usize, f64> = HashMap::new();

        for pool in &graph.pools {
            for lane in pool.get_lanes() {
                for node in lane.get_layers() {
                    // Skip marker and internal loop nodes (same rules as the drawing loop below)
                    let mut is_marker = false;
                    let mut is_inside_loop = false;
                    for layout in &loop_layouts {
                        if node.id == layout.region.entry_id || node.id == layout.region.exit_id {
                            is_marker = true;
                        }
                        if layout.region.node_ids.contains(&node.id) {
                            is_inside_loop = true;
                        }
                    }
                    if is_marker || is_inside_loop {
                        continue;
                    }

                    let layer = node.layer_id.unwrap_or(0);
                    layer_ids.insert(layer);
                    // store a representative x for base alignment
                    let (x, _y) = di_xy_for_node(node, &loop_layouts);
                    sample_x_by_layer.entry(layer).or_insert(x);
                }
            }
        }

        if !layer_ids.is_empty() {
            let layer_to_col: HashMap<usize, usize> = layer_ids
                .iter()
                .copied()
                .enumerate()
                .map(|(col, layer)| (layer, col))
                .collect();

            // Use the smallest layer's current x as the base.
            let first_layer = *layer_ids.iter().next().unwrap();
            let base_x = *sample_x_by_layer.get(&first_layer).unwrap_or(&0.0);
            let layer_width = 260.0;

            for pool in &graph.pools {
                for lane in pool.get_lanes() {
                    for node in lane.get_layers() {
                        let mut is_marker = false;
                        let mut is_inside_loop = false;
                        for layout in &loop_layouts {
                            if node.id == layout.region.entry_id || node.id == layout.region.exit_id
                            {
                                is_marker = true;
                            }
                            if layout.region.node_ids.contains(&node.id) {
                                is_inside_loop = true;
                            }
                        }
                        if is_marker || is_inside_loop {
                            continue;
                        }

                        let layer = node.layer_id.unwrap_or(0);
                        if let Some(col) = layer_to_col.get(&layer) {
                            top_level_x_override.insert(node.id, base_x + (*col as f64) * layer_width);
                        }
                    }
                }
            }
        }
    }
    for pool in &graph.pools {
        for lane in pool.get_lanes() {
            for node in lane.get_layers() {
                // Skip loop marker tasks (they are not emitted into BPMN)
                let mut is_marker = false;
                for layout in &loop_layouts {
                    if node.id == layout.region.entry_id || node.id == layout.region.exit_id {
                        is_marker = true;
                        break;
                    }
                }
                if is_marker {
                    continue;
                }

                // If we have a loop subprocess, do NOT draw the internal loop-body nodes
                // on the top-level diagram (the subprocess is collapsed there).
                let mut is_inside_loop = false;
                for layout in &loop_layouts {
                    if layout.region.node_ids.contains(&node.id) {
                        is_inside_loop = true;
                        break;
                    }
                }
                if is_inside_loop {
                    continue;
                }

                let (width, height) = if let Some(event) = &node.event {
                    get_node_size(event)
                } else {
                    (100, 80)
                };

                let (mut x, y) = di_xy_for_node(node, &loop_layouts);
                if let Some(ox) = top_level_x_override.get(&node.id) {
                    x = *ox;
                }

                bpmn.push_str(&format!(
                    r#"<bpmndi:BPMNShape id="{}_di" bpmnElement="{}">
                <dc:Bounds x="{:.2}" y="{:.2}" width="{}" height="{}" />
                </bpmndi:BPMNShape>"#,
                    get_node_bpmn_id(node),
                    get_node_bpmn_id(node),
                    x,
                    y,
                    width,
                    height
                ));
            }
        }
    }

    // Add BPMN shape for the loop subprocess (collapsed), if present.
    for pool_name in &pool_names {
        let pool_nodes: Vec<&Node> = graph.get_nodes_by_pool_name(pool_name);
        if let Some(region) = detect_loop_region(&pool_nodes, graph) {
            let (sx, sy, sw, sh) = region_bounds_collapsed(graph, &region);
            bpmn.push_str(&format!(
                r#"<bpmndi:BPMNShape id="SubProcess_Loop_di" bpmnElement="SubProcess_Loop" isExpanded="false">
                <dc:Bounds x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" />
                </bpmndi:BPMNShape>"#,
                sx, sy, sw, sh
            ));
        }
    }

    // Add BPMN edges for sequence flows
    for edge in &graph.edges {
        // Skip edges that originate from / terminate at loop marker tasks.
        // These marker tasks are not emitted into BPMN; the subprocess boundary represents entry/exit.
        let mut skip_marker_edge = false;
        for layout in &loop_layouts {
            if edge.from == layout.region.entry_id || edge.to == layout.region.exit_id {
                skip_marker_edge = true;
                break;
            }
            // Also skip any edge fully inside the loop-body on the top-level diagram.
            if layout.region.node_ids.contains(&edge.from) || layout.region.node_ids.contains(&edge.to) {
                skip_marker_edge = true;
                break;
            }
        }
        if skip_marker_edge {
            continue;
        }

        bpmn.push_str(&format!(
            r#"<bpmndi:BPMNEdge id="Flow_{}_{}_di" bpmnElement="Flow_{}_{}">"#,
            edge.from, edge.to, edge.from, edge.to
        ));

        // Route edges Camunda-like:
        // - if on same row -> straight (2 points)
        // - if crossing rows -> orthogonal with the vertical segment near the TARGET
        //   (prevents long "cutting" diagonals through the diagram)
        if let (Some(from_node), Some(to_node)) =
            (graph.get_node_by_id(edge.from), graph.get_node_by_id(edge.to))
        {
            if let (Some(from_event), Some(to_event)) = (&from_node.event, &to_node.event) {
                let (from_w, from_h) = get_node_size(from_event);
                let (_to_w, to_h) = get_node_size(to_event);

                let (mut from_x, from_y) = di_xy_for_node(from_node, &loop_layouts);
                let (mut to_x, to_y) = di_xy_for_node(to_node, &loop_layouts);
                if let Some(ox) = top_level_x_override.get(&from_node.id) {
                    from_x = *ox;
                }
                if let Some(ox) = top_level_x_override.get(&to_node.id) {
                    to_x = *ox;
                }

                let start_x = from_x + from_w as f64;
                let start_y = from_y + (from_h as f64 / 2.0);
                let end_x = to_x;
                let end_y = to_y + (to_h as f64 / 2.0);

                let dy = (end_y - start_y).abs();
                let force_orthogonal =
                    is_gateway_event(from_event) && dy > 8.0 && end_x > start_x + 60.0;

                // Place waypoints slightly outside shapes to avoid "sticking" into the body.
                let pad = 8.0;
                let start_x = start_x + pad;
                let end_x = (end_x - pad).max(start_x + 10.0);

                if (!force_orthogonal && dy <= 25.0) || end_x <= start_x + 40.0 {
                    // straight
                    bpmn.push_str(&format!(
                        r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                        start_x, start_y
                    ));
                    bpmn.push_str(&format!(
                        r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                        end_x, end_y
                    ));
                } else {
                    // orthogonal, vertical near target (no corridor detours)
                    let mid_x = (end_x - 30.0).max(start_x + 30.0);
                    bpmn.push_str(&format!(
                        r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                        start_x, start_y
                    ));
                    bpmn.push_str(&format!(
                        r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                        mid_x, start_y
                    ));
                    bpmn.push_str(&format!(
                        r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                        mid_x, end_y
                    ));
                    bpmn.push_str(&format!(
                        r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                        end_x, end_y
                    ));
                }
            }
        }

        bpmn.push_str(r#"</bpmndi:BPMNEdge>"#);
    }

    // Add DI edges for synthetic subprocess flows (SubStart->first, last->SubEnd)
    // Top-level diagram keeps the subprocess collapsed; do not draw synthetic inner flows here.

    // Close TOP-LEVEL plane/diagram first.
    bpmn.push_str(
        r#"  </bpmndi:BPMNPlane>
</bpmndi:BPMNDiagram>
"#,
    );

    // If we have a loop region, add a dedicated subprocess diagram (Camunda-style drill-down).
    for pool_name in &pool_names {
        let pool_nodes: Vec<&Node> = graph.get_nodes_by_pool_name(pool_name);
        if let Some(region) = detect_loop_region(&pool_nodes, graph) {
            bpmn.push_str(
                r#"<bpmndi:BPMNDiagram id="BPMNDiagram_2">
  <bpmndi:BPMNPlane id="BPMNPlane_2" bpmnElement="SubProcess_Loop">
"#,
            );

            // Shapes: SubStart/SubEnd + all internal nodes
            // SubStart/SubEnd placement based on first/last internal nodes.
            let mut substart_x = 200.0;
            let mut substart_y = 120.0;
            let mut subend_x = 560.0;
            let mut subend_y = 120.0;

            if let Some(first_edge) = graph.edges.iter().find(|e| e.from == region.entry_id) {
                if let Some(first) = graph.get_node_by_id(first_edge.to) {
                    if let Some(first_ev) = &first.event {
                        let (_fw, fh) = get_node_size(first_ev);
                        let (fx, fy) = di_xy_for_node(first, &loop_layouts);
                        substart_x = (fx - 60.0).max(120.0);
                        substart_y = (fy + (fh as f64 / 2.0) - 18.0).max(60.0);
                    }
                }
            }

            if let Some(last_edge) = graph.edges.iter().find(|e| e.to == region.exit_id) {
                if let Some(last) = graph.get_node_by_id(last_edge.from) {
                    if let Some(last_ev) = &last.event {
                        let (lw, lh) = get_node_size(last_ev);
                        let (lx, ly) = di_xy_for_node(last, &loop_layouts);
                        subend_x = (lx + lw as f64 + 80.0).max(substart_x + 220.0);
                        subend_y = (ly + (lh as f64 / 2.0) - 18.0).max(60.0);
                    }
                }
            }

            bpmn.push_str(&format!(
                r#"<bpmndi:BPMNShape id="SubStart_di" bpmnElement="SubStart">
                <dc:Bounds x="{:.2}" y="{:.2}" width="36" height="36" />
                </bpmndi:BPMNShape>"#,
                substart_x, substart_y
            ));
            bpmn.push_str(&format!(
                r#"<bpmndi:BPMNShape id="SubEnd_di" bpmnElement="SubEnd">
                <dc:Bounds x="{:.2}" y="{:.2}" width="36" height="36" />
                </bpmndi:BPMNShape>"#,
                subend_x, subend_y
            ));

            // Internal node shapes
            for id in region.node_ids.iter().copied() {
                let Some(node) = graph.get_node_by_id(id) else { continue };
                let Some(ev) = &node.event else { continue };
                let (w, h) = get_node_size(ev);
                let (x, y) = di_xy_for_node(node, &loop_layouts);
                bpmn.push_str(&format!(
                    r#"<bpmndi:BPMNShape id="{}_di" bpmnElement="{}">
                <dc:Bounds x="{:.2}" y="{:.2}" width="{}" height="{}" />
                </bpmndi:BPMNShape>"#,
                    get_node_bpmn_id(node),
                    get_node_bpmn_id(node),
                    x,
                    y,
                    w,
                    h
                ));
            }

            // Internal edges (only those fully inside the subprocess)
            for edge in &graph.edges {
                if !region.contains(edge.from) || !region.contains(edge.to) {
                    continue;
                }
                // entry/exit marker tasks are not drawn
                if edge.from == region.entry_id || edge.to == region.exit_id {
                    continue;
                }

                bpmn.push_str(&format!(
                    r#"<bpmndi:BPMNEdge id="Flow_{}_{}_di_2" bpmnElement="Flow_{}_{}">"#,
                    edge.from, edge.to, edge.from, edge.to
                ));

                if let (Some(from_node), Some(to_node)) =
                    (graph.get_node_by_id(edge.from), graph.get_node_by_id(edge.to))
                {
                    if let (Some(from_event), Some(to_event)) = (&from_node.event, &to_node.event) {
                        let (from_w, from_h) = get_node_size(from_event);
                        let (_to_w, to_h) = get_node_size(to_event);
                        let (from_x, from_y) = di_xy_for_node(from_node, &loop_layouts);
                        let (to_x, to_y) = di_xy_for_node(to_node, &loop_layouts);
                        let mut start_x = from_x + from_w as f64;
                        let start_y = from_y + (from_h as f64 / 2.0);
                        let mut end_x = to_x;
                        let end_y = to_y + (to_h as f64 / 2.0);

                        let dy = (end_y - start_y).abs();
                        let force_orthogonal =
                            is_gateway_event(from_event) && dy > 8.0 && end_x > start_x + 60.0;
                        let pad = 8.0;
                        start_x += pad;
                        end_x = (end_x - pad).max(start_x + 10.0);

                        if (!force_orthogonal && dy <= 25.0) || end_x <= start_x + 40.0 {
                            bpmn.push_str(&format!(
                                r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                                start_x, start_y
                            ));
                            bpmn.push_str(&format!(
                                r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                                end_x, end_y
                            ));
                        } else {
                            let mid_x = (end_x - 30.0).max(start_x + 30.0);
                            bpmn.push_str(&format!(
                                r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                                start_x, start_y
                            ));
                            bpmn.push_str(&format!(
                                r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                                mid_x, start_y
                            ));
                            bpmn.push_str(&format!(
                                r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                                mid_x, end_y
                            ));
                            bpmn.push_str(&format!(
                                r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                                end_x, end_y
                            ));
                        }
                    }
                }

                bpmn.push_str(r#"</bpmndi:BPMNEdge>"#);
            }

            // Synthetic SubStart -> first / last -> SubEnd edges (for drill-down view)
            if let Some(first_edge) = graph.edges.iter().find(|e| e.from == region.entry_id) {
                if let Some(first) = graph.get_node_by_id(first_edge.to) {
                    if region.node_ids.contains(&first.id) {
                        bpmn.push_str(&format!(
                            r#"<bpmndi:BPMNEdge id="Flow_SubStart_{}_di_2" bpmnElement="Flow_SubStart_{}">"#,
                            first.id, first.id
                        ));
                        bpmn.push_str(&format!(
                            r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                            substart_x + 36.0,
                            substart_y + 18.0
                        ));
                        let (fx, fy) = di_xy_for_node(first, &loop_layouts);
                        bpmn.push_str(&format!(
                            r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                            fx,
                            fy + 40.0
                        ));
                        bpmn.push_str(r#"</bpmndi:BPMNEdge>"#);
                    }
                }
            }

            if let Some(last_edge) = graph.edges.iter().find(|e| e.to == region.exit_id) {
                if let Some(last) = graph.get_node_by_id(last_edge.from) {
                    if region.node_ids.contains(&last.id) {
                        bpmn.push_str(&format!(
                            r#"<bpmndi:BPMNEdge id="Flow_{}_SubEnd_di_2" bpmnElement="Flow_{}_SubEnd">"#,
                            last.id, last.id
                        ));
                        let (lx, ly) = di_xy_for_node(last, &loop_layouts);
                        let (lw, lh) = last
                            .event
                            .as_ref()
                            .map(get_node_size)
                            .unwrap_or((100, 80));
                        bpmn.push_str(&format!(
                            r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                            lx + lw as f64,
                            ly + (lh as f64 / 2.0)
                        ));
                        bpmn.push_str(&format!(
                            r#"<di:waypoint x="{:.2}" y="{:.2}" />"#,
                            subend_x,
                            subend_y + 18.0
                        ));
                        bpmn.push_str(r#"</bpmndi:BPMNEdge>"#);
                    }
                }
            }

            bpmn.push_str(
                r#"  </bpmndi:BPMNPlane>
</bpmndi:BPMNDiagram>
"#,
            );
        }
    }

    // Close definitions at the very end.
    bpmn.push_str(r#"</bpmn:definitions>"#);

    bpmn
}

#[derive(Clone)]
struct LoopRegion {
    entry_id: usize,
    exit_id: usize,
    // Node IDs that are inside the subprocess (excluding entry/exit marker tasks)
    node_ids: HashSet<usize>,
}

impl LoopRegion {
    fn contains(&self, id: usize) -> bool {
        id == self.entry_id || id == self.exit_id || self.node_ids.contains(&id)
    }
}

struct LoopLayout {
    region: LoopRegion,
    sx: f64,
    sy: f64,
    min_layer: usize,
    entry_y: f64,
}

fn compute_loop_layouts(graph: &Graph, pool_ids: &HashSet<String>) -> Vec<LoopLayout> {
    let mut out = Vec::new();
    for pool_id in pool_ids {
        let pool_nodes: Vec<&Node> = graph.get_nodes_by_pool_name(pool_id);
        if let Some(region) = detect_loop_region(&pool_nodes, graph) {
            let (sx, sy, _sw, _sh) = region_bounds_collapsed(graph, &region);
            let min_layer = region
                .node_ids
                .iter()
                .filter_map(|id| graph.get_node_by_id(*id))
                .filter_map(|n| n.layer_id)
                .min()
                .unwrap_or(0);
            let entry_y = graph
                .get_node_by_id(region.entry_id)
                .and_then(|n| n.y)
                .unwrap_or(0.0);
            out.push(LoopLayout {
                region,
                sx,
                sy,
                min_layer,
                entry_y,
            });
        }
    }
    out
}

fn di_xy_for_node(node: &Node, layouts: &Vec<LoopLayout>) -> (f64, f64) {
    // Default: use computed layout positions stored on node
    let mut x = node.x.unwrap_or(0.0) + node.x_offset.unwrap_or(0.0);
    let mut y = node.y.unwrap_or(0.0) + node.y_offset.unwrap_or(0.0);

    for layout in layouts {
        if layout.region.node_ids.contains(&node.id) {
            let layer = node
                .layer_id
                .unwrap_or(layout.min_layer)
                .saturating_sub(layout.min_layer);
            // Spread nodes inside subprocess more (Camunda-like): multiple rows and wider columns.
            // Use the original computed `node.y` to preserve branch separation.
            let base_y = node.y.unwrap_or(layout.entry_y);
            let dy = (base_y - layout.entry_y).max(0.0);
            // Stronger vertical separation between branches inside the loop (Camunda-like).
            let row = (dy / 120.0).floor().min(6.0);

            // Inner subprocess layout (more roomy, closer to previous "good" version)
            x = layout.sx + 120.0 + (layer as f64) * 220.0;
            y = layout.sy + 60.0 + row * 220.0;
            break;
        }
    }

    (x, y)
}

fn detect_loop_region(pool_nodes: &Vec<&Node>, graph: &Graph) -> Option<LoopRegion> {
    let entry = pool_nodes.iter().find(|n| match &n.event {
        Some(BpmnEvent::ActivityTask(label)) => {
            strip_step_prefix(label).trim_start().starts_with("Loop:")
        }
        _ => false,
    })?;
    let exit = pool_nodes.iter().find(|n| match &n.event {
        Some(BpmnEvent::ActivityTask(label)) => strip_step_prefix(label).trim() == "End Loop",
        _ => false,
    })?;

    // Collect nodes on a forward walk from entry to exit (by id), including branches.
    // This is a conservative approximation for typical "loop body is a contiguous block".
    let mut visited: HashSet<usize> = HashSet::new();
    let mut stack: Vec<usize> = vec![entry.id];
    while let Some(cur) = stack.pop() {
        if !visited.insert(cur) {
            continue;
        }
        if cur == exit.id {
            continue;
        }
        for e in graph.edges.iter().filter(|e| e.from == cur) {
            stack.push(e.to);
        }
    }

    // Remove entry/exit marker tasks from inner nodes
    visited.remove(&entry.id);
    visited.remove(&exit.id);

    Some(LoopRegion {
        entry_id: entry.id,
        exit_id: exit.id,
        node_ids: visited,
    })
}

fn region_bounds_collapsed(graph: &Graph, region: &LoopRegion) -> (f64, f64, f64, f64) {
    // For a collapsed subprocess we want a compact block, not a huge bounding box
    // around all internal nodes. Anchor it at the "Loop:" marker task position.
    if let Some(entry) = graph.get_node_by_id(region.entry_id) {
        let x = entry.x.unwrap_or(0.0) + entry.x_offset.unwrap_or(0.0);
        let y = entry.y.unwrap_or(0.0) + entry.y_offset.unwrap_or(0.0);
        // Keep collapsed subprocess the same size as a regular task box.
        // (Camunda will still render the collapsed subprocess marker + title.)
        return (x, y, 100.0, 80.0);
    }
    (200.0, 120.0, 100.0, 80.0)
}

fn generate_multi_instance_subprocess(bpmn: &mut String, graph: &Graph, region: &LoopRegion) {
    // Build a subprocess with a synthetic start/end and route edges inside it.
    // We'll reference existing node ids inside, but nest them under SubProcess_Loop.
    bpmn.push_str(r#"<bpmn:subProcess id="SubProcess_Loop" name="Для каждого займа">"#);
    // Camunda-style MI settings (editable in UI)
    bpmn.push_str(
        r#"<bpmn:multiInstanceLoopCharacteristics isSequential="true" camunda:collection="loans" camunda:elementVariable="loan" />"#,
    );

    // Start/end events inside the subprocess (BPMN-correct)
    bpmn.push_str(r#"<bpmn:startEvent id="SubStart" name="Start" />"#);
    bpmn.push_str(r#"<bpmn:endEvent id="SubEnd" name="End" />"#);

    // Emit inner nodes (skip marker tasks)
    let mut inner_nodes: Vec<&Node> = Vec::new();
    for id in region.node_ids.iter().copied() {
        if let Some(n) = graph.get_node_by_id(id) {
            inner_nodes.push(n);
        }
    }
    inner_nodes.sort_by_key(|n| n.id);

    for node in &inner_nodes {
        generate_flow_node_without_flows(bpmn, node);
    }

    // Emit inner flows: include edges whose endpoints are inside region.node_ids
    for edge in graph.edges.iter() {
        if region.node_ids.contains(&edge.from) && region.node_ids.contains(&edge.to) {
            let from_node = graph.get_node_by_id(edge.from).unwrap();
            let to_node = graph.get_node_by_id(edge.to).unwrap();
            if let Some(text) = &edge.text {
                bpmn.push_str(&format!(
                    r#"<bpmn:sequenceFlow id="Flow_{}_{}" name="{}" sourceRef="{}" targetRef="{}" />"#,
                    edge.from,
                    edge.to,
                    escape_xml_attr(text),
                    get_node_bpmn_id(from_node),
                    get_node_bpmn_id(to_node)
                ));
            } else {
                bpmn.push_str(&format!(
                    r#"<bpmn:sequenceFlow id="Flow_{}_{}" sourceRef="{}" targetRef="{}" />"#,
                    edge.from,
                    edge.to,
                    get_node_bpmn_id(from_node),
                    get_node_bpmn_id(to_node)
                ));
            }
        }
    }

    // Connect SubStart -> first node (successor of entry marker)
    if let Some(first_edge) = graph.edges.iter().find(|e| e.from == region.entry_id) {
        if let Some(first) = graph.get_node_by_id(first_edge.to) {
            bpmn.push_str(&format!(
                r#"<bpmn:sequenceFlow id="Flow_SubStart_{}" sourceRef="SubStart" targetRef="{}" />"#,
                first.id,
                get_node_bpmn_id(first)
            ));
        }
    }

    // Connect last node (predecessor of exit marker) -> SubEnd
    if let Some(last_edge) = graph.edges.iter().find(|e| e.to == region.exit_id) {
        if let Some(last) = graph.get_node_by_id(last_edge.from) {
            bpmn.push_str(&format!(
                r#"<bpmn:sequenceFlow id="Flow_{}_SubEnd" sourceRef="{}" targetRef="SubEnd" />"#,
                last.id,
                get_node_bpmn_id(last)
            ));
        }
    }

    bpmn.push_str(r#"</bpmn:subProcess>"#);
}

fn generate_flow_node_without_flows(bpmn: &mut String, node: &Node) {
    if let Some(event) = &node.event {
        match event {
            BpmnEvent::Start(label)
            | BpmnEvent::StartTimerEvent(label)
            | BpmnEvent::StartSignalEvent(label)
            | BpmnEvent::StartMessageEvent(label)
            | BpmnEvent::StartConditionalEvent(label) => {
                bpmn.push_str(&format!(
                    r#"<bpmn:startEvent id="{}" name="{}" />"#,
                    get_node_bpmn_id(node),
                    escape_xml_attr(label)
                ));
            }
            BpmnEvent::End(label)
            | BpmnEvent::EndErrorEvent(label)
            | BpmnEvent::EndCancelEvent(label)
            | BpmnEvent::EndSignalEvent(label)
            | BpmnEvent::EndMessageEvent(label)
            | BpmnEvent::EndTerminateEvent(label)
            | BpmnEvent::EndEscalationEvent(label)
            | BpmnEvent::EndCompensationEvent(label) => {
                bpmn.push_str(&format!(
                    r#"<bpmn:endEvent id="{}" name="{}" />"#,
                    get_node_bpmn_id(node),
                    escape_xml_attr(label)
                ));
            }
            BpmnEvent::ActivityTask(label)
            | BpmnEvent::ActivitySubprocess(label)
            | BpmnEvent::ActivityCallActivity(label)
            | BpmnEvent::ActivityEventSubprocess(label)
            | BpmnEvent::ActivityTransaction(label)
            | BpmnEvent::TaskUser(label)
            | BpmnEvent::TaskService(label)
            | BpmnEvent::TaskBusinessRule(label)
            | BpmnEvent::TaskScript(label) => {
                let element_type = match event {
                    BpmnEvent::ActivityTask(_) => classify_task_element_type(label),
                    BpmnEvent::TaskUser(_) => "userTask",
                    BpmnEvent::TaskService(_) => "serviceTask",
                    BpmnEvent::TaskBusinessRule(_) => "businessRuleTask",
                    BpmnEvent::TaskScript(_) => "scriptTask",
                    BpmnEvent::ActivitySubprocess(_) => "subProcess",
                    BpmnEvent::ActivityCallActivity(_) => "callActivity",
                    BpmnEvent::ActivityEventSubprocess(_) => "subProcess triggeredByEvent=\"true\"",
                    BpmnEvent::ActivityTransaction(_) => "transaction",
                    _ => "task",
                };

                bpmn.push_str(&format!(
                    r#"<bpmn:{} id="{}" name="{}" />"#,
                    element_type,
                    get_node_bpmn_id(node),
                    escape_xml_attr(strip_step_prefix(label))
                ));
            }
            BpmnEvent::GatewayExclusive | BpmnEvent::GatewayInclusive | BpmnEvent::GatewayJoin(_) => {
                let element_type = match event {
                    BpmnEvent::GatewayExclusive => "exclusiveGateway",
                    BpmnEvent::GatewayInclusive => "inclusiveGateway",
                    BpmnEvent::GatewayJoin(_) => "parallelGateway",
                    _ => "exclusiveGateway",
                };
                bpmn.push_str(&format!(
                    r#"<bpmn:{} id="{}" />"#,
                    element_type,
                    get_node_bpmn_id(node)
                ));
            }
            _ => {}
        }
    }
}

fn generate_flow_node(bpmn: &mut String, node: &Node, graph: &Graph) {
    if let Some(event) = &node.event {
        match event {
            // Start Events
            BpmnEvent::Start(label)
            | BpmnEvent::StartTimerEvent(label)
            | BpmnEvent::StartSignalEvent(label)
            | BpmnEvent::StartMessageEvent(label)
            | BpmnEvent::StartConditionalEvent(label) => {
                bpmn.push_str(&format!(
                    r#"<bpmn:startEvent id="{}" name="{}">"#,
                    get_node_bpmn_id(node),
                    label
                ));

                // Add outgoing flows
                for edge in graph.edges.iter().filter(|e| e.from == node.id) {
                    bpmn.push_str(&format!(
                        r#"<bpmn:outgoing>Flow_{}_{}</bpmn:outgoing>"#,
                        edge.from, edge.to
                    ));
                }

                bpmn.push_str(r#"</bpmn:startEvent>"#);
            }

            // End Events
            BpmnEvent::End(label)
            | BpmnEvent::EndErrorEvent(label)
            | BpmnEvent::EndCancelEvent(label)
            | BpmnEvent::EndSignalEvent(label)
            | BpmnEvent::EndMessageEvent(label)
            | BpmnEvent::EndTerminateEvent(label)
            | BpmnEvent::EndEscalationEvent(label)
            | BpmnEvent::EndCompensationEvent(label) => {
                bpmn.push_str(&format!(
                    r#"<bpmn:endEvent id="{}" name="{}">"#,
                    get_node_bpmn_id(node),
                    label
                ));

                // Add incoming flows
                for edge in graph.edges.iter().filter(|e| e.to == node.id) {
                    bpmn.push_str(&format!(
                        r#"<bpmn:incoming>Flow_{}_{}</bpmn:incoming>"#,
                        edge.from, edge.to
                    ));
                }

                bpmn.push_str(r#"</bpmn:endEvent>"#);
            }

            // Tasks and Activities
            BpmnEvent::ActivityTask(label)
            | BpmnEvent::ActivitySubprocess(label)
            | BpmnEvent::ActivityCallActivity(label)
            | BpmnEvent::ActivityEventSubprocess(label)
            | BpmnEvent::ActivityTransaction(label)
            | BpmnEvent::TaskUser(label)
            | BpmnEvent::TaskService(label)
            | BpmnEvent::TaskBusinessRule(label)
            | BpmnEvent::TaskScript(label) => {
                let element_type = match event {
                    BpmnEvent::ActivityTask(_) => classify_task_element_type(label),
                    BpmnEvent::TaskUser(_) => "userTask",
                    BpmnEvent::TaskService(_) => "serviceTask",
                    BpmnEvent::TaskBusinessRule(_) => "businessRuleTask",
                    BpmnEvent::TaskScript(_) => "scriptTask",
                    BpmnEvent::ActivitySubprocess(_) => "subProcess",
                    BpmnEvent::ActivityCallActivity(_) => "callActivity",
                    BpmnEvent::ActivityEventSubprocess(_) => "subProcess triggeredByEvent=\"true\"",
                    BpmnEvent::ActivityTransaction(_) => "transaction",
                    _ => "task",
                };

                bpmn.push_str(&format!(
                    r#"<bpmn:{} id="{}" name="{}">"#,
                    element_type,
                    get_node_bpmn_id(node),
                    escape_xml_attr(strip_step_prefix(label))
                ));

                // Add incoming flows
                for edge in graph.edges.iter().filter(|e| e.to == node.id) {
                    bpmn.push_str(&format!(
                        r#"<bpmn:incoming>Flow_{}_{}</bpmn:incoming>"#,
                        edge.from, edge.to
                    ));
                }

                // Add outgoing flows
                for edge in graph.edges.iter().filter(|e| e.from == node.id) {
                    bpmn.push_str(&format!(
                        r#"<bpmn:outgoing>Flow_{}_{}</bpmn:outgoing>"#,
                        edge.from, edge.to
                    ));
                }

                bpmn.push_str(&format!(r#"</bpmn:{}>"#, element_type));
            }

            // Gateways
            BpmnEvent::GatewayExclusive
            | BpmnEvent::GatewayInclusive
            | BpmnEvent::GatewayJoin(_) => {
                let element_type = match event {
                    BpmnEvent::GatewayExclusive => "exclusiveGateway",
                    BpmnEvent::GatewayInclusive => "inclusiveGateway",
                    BpmnEvent::GatewayJoin(_) => "parallelGateway",
                    _ => "exclusiveGateway",
                };

                bpmn.push_str(&format!(
                    r#"<bpmn:{} id="{}">"#,
                    element_type,
                    get_node_bpmn_id(node),
                ));

                // Add incoming flows
                for edge in graph.edges.iter().filter(|e| e.to == node.id) {
                    bpmn.push_str(&format!(
                        r#"<bpmn:incoming>Flow_{}_{}</bpmn:incoming>"#,
                        edge.from, edge.to
                    ));
                }

                // Add outgoing flows
                for edge in graph.edges.iter().filter(|e| e.from == node.id) {
                    bpmn.push_str(&format!(
                        r#"<bpmn:outgoing>Flow_{}_{}</bpmn:outgoing>"#,
                        edge.from, edge.to
                    ));
                }

                bpmn.push_str(&format!(r#"</bpmn:{}>"#, element_type));
            }

            // Data Objects
            BpmnEvent::DataStoreReference(label) => {
                bpmn.push_str(&format!(
                    r#"<bpmn:dataStoreReference id="{}" name="{}" />"#,
                    get_node_bpmn_id(node),
                    label
                ));
            }
            BpmnEvent::DataObjectReference(label) => {
                bpmn.push_str(&format!(
                    r#"<bpmn:dataObjectReference id="{}" name="{}" />"#,
                    get_node_bpmn_id(node),
                    label
                ));
            }
            _ => {}
        }
    }
}

fn classify_task_element_type(label: &str) -> &'static str {
    let l = label.trim().to_lowercase();

    // Explicit type prefixes from DSL agent: [API]/[SCRIPT]/[MANUAL]/[AUTO]/[DB]/[MSG]
    if l.starts_with("[api]") || l.starts_with("[db]") || l.starts_with("[msg]") {
        return "serviceTask";
    }
    if l.starts_with("[script]") {
        return "scriptTask";
    }
    if l.starts_with("[manual]") {
        return "manualTask";
    }
    if l.starts_with("[auto]") {
        return "task";
    }
    // API calls -> service task
    if l.starts_with("get ")
        || l.starts_with("post ")
        || l.starts_with("put ")
        || l.starts_with("delete ")
        || l.contains(" post ")
        || l.contains(": post")
        || l.contains("post /")
        || l.contains("get /")
        || l.contains("/api")
        || l.contains("http")
    {
        return "serviceTask";
    }
    // Calculations -> script task
    if l.starts_with("рассчитать") || l.contains("calculate") || l.contains("amount_to_") {
        return "scriptTask";
    }
    // Skips/manual actions
    if l.starts_with("пропустить") || l.contains("skip") {
        return "manualTask";
    }
    // Registrations typically backend integration
    if l.contains("регистрация") || l.contains("register") {
        return "serviceTask";
    }
    "task"
}

fn strip_step_prefix(label: &str) -> &str {
    let s = label.trim_start();
    for p in [
        "[API]", "[SCRIPT]", "[MANUAL]", "[AUTO]", "[DB]", "[MSG]", "[api]", "[script]", "[manual]",
        "[auto]", "[db]", "[msg]",
    ] {
        if let Some(rest) = s.strip_prefix(p) {
            return rest.trim_start();
        }
    }
    s
}

fn generate_sequence_flows(
    bpmn: &mut String,
    graph: &Graph,
    pool_nodes: &Vec<&Node>,
    loop_region: Option<&LoopRegion>,
) {
    let node_ids: HashSet<usize> = pool_nodes.iter().map(|node| node.id).collect();

    for edge in &graph.edges {
        if node_ids.contains(&edge.from) && node_ids.contains(&edge.to) {
            if let Some(region) = loop_region {
                // Skip flows fully inside the loop region: those are nested in the subprocess.
                if region.contains(edge.from) && region.contains(edge.to) {
                    continue;
                }
                // Redirect edges that enter/exit the loop marker tasks to the subprocess boundary.
                // entry marker incoming -> SubProcess_Loop
                // exit marker outgoing -> from SubProcess_Loop
            }
            let from_node = graph.get_node_by_id(edge.from).unwrap();
            let to_node = graph.get_node_by_id(edge.to).unwrap();

            let mut source_ref = get_node_bpmn_id(from_node);
            let mut target_ref = get_node_bpmn_id(to_node);
            if let Some(region) = loop_region {
                if edge.to == region.entry_id {
                    target_ref = "SubProcess_Loop".to_string();
                }
                if edge.from == region.exit_id {
                    source_ref = "SubProcess_Loop".to_string();
                }
                // Skip the marker tasks themselves by not emitting their direct flows
                if edge.from == region.entry_id || edge.to == region.exit_id {
                    continue;
                }
            }

            // Lisa sequenceFlow element
            if let Some(text) = &edge.text {
                bpmn.push_str(&format!(
                    r#"<bpmn:sequenceFlow id="Flow_{}_{}" name="{}" sourceRef="{}" targetRef="{}" />"#,
                    edge.from,
                    edge.to,
                    escape_xml_attr(text),
                    source_ref,
                    target_ref
                ));
            } else {
                bpmn.push_str(&format!(
                    r#"<bpmn:sequenceFlow id="Flow_{}_{}" sourceRef="{}" targetRef="{}" />"#,
                    edge.from, edge.to, source_ref, target_ref
                ));
            }
        }
    }
}

fn escape_xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}

fn get_node_bpmn_id(node: &Node) -> String {
    if let Some(event) = &node.event {
        match event {
            BpmnEvent::Start(_)
            | BpmnEvent::StartTimerEvent(_)
            | BpmnEvent::StartSignalEvent(_)
            | BpmnEvent::StartMessageEvent(_)
            | BpmnEvent::StartConditionalEvent(_) => format!("StartEvent_{}", node.id),

            BpmnEvent::End(_)
            | BpmnEvent::EndErrorEvent(_)
            | BpmnEvent::EndCancelEvent(_)
            | BpmnEvent::EndSignalEvent(_)
            | BpmnEvent::EndMessageEvent(_)
            | BpmnEvent::EndTerminateEvent(_)
            | BpmnEvent::EndEscalationEvent(_)
            | BpmnEvent::EndCompensationEvent(_) => {
                format!("EndEvent_{}", node.id)
            }

            BpmnEvent::ActivityTask(_)
            | BpmnEvent::TaskUser(_)
            | BpmnEvent::TaskService(_)
            | BpmnEvent::TaskBusinessRule(_)
            | BpmnEvent::TaskScript(_) => format!("Activity_{}", node.id),

            BpmnEvent::ActivitySubprocess(_) => format!("SubProcess_{}", node.id),
            BpmnEvent::ActivityCallActivity(_) => format!("CallActivity_{}", node.id),
            BpmnEvent::ActivityEventSubprocess(_) => format!("EventSubProcess_{}", node.id),
            BpmnEvent::ActivityTransaction(_) => format!("Transaction_{}", node.id),

            BpmnEvent::GatewayExclusive
            | BpmnEvent::GatewayInclusive
            | BpmnEvent::GatewayJoin(_) => format!("Gateway_{}", node.id),

            BpmnEvent::DataStoreReference(_) => format!("DataStoreReference_{}", node.id),
            BpmnEvent::DataObjectReference(_) => format!("DataObjectReference_{}", node.id),

            // Add other event types as needed
            _ => format!("Node_{}", node.id),
        }
    } else {
        format!("Node_{}", node.id)
    }
}

pub fn get_node_size(event: &BpmnEvent) -> (usize, usize) {
    match event {
        // Start Events
        BpmnEvent::Start(_)
        | BpmnEvent::StartTimerEvent(_)
        | BpmnEvent::StartSignalEvent(_)
        | BpmnEvent::StartMessageEvent(_)
        | BpmnEvent::StartConditionalEvent(_) => (36, 36),

        // End Events
        BpmnEvent::End(_)
        | BpmnEvent::EndErrorEvent(_)
        | BpmnEvent::EndCancelEvent(_)
        | BpmnEvent::EndSignalEvent(_)
        | BpmnEvent::EndMessageEvent(_)
        | BpmnEvent::EndTerminateEvent(_)
        | BpmnEvent::EndEscalationEvent(_)
        | BpmnEvent::EndCompensationEvent(_) => (36, 36),

        // Gateways
        BpmnEvent::GatewayExclusive | BpmnEvent::GatewayInclusive | BpmnEvent::GatewayJoin(_) => {
            (50, 50)
        }

        // Activities
        BpmnEvent::ActivityTask(_)
        | BpmnEvent::ActivityCallActivity(_)
        | BpmnEvent::TaskUser(_)
        | BpmnEvent::TaskService(_)
        | BpmnEvent::TaskBusinessRule(_)
        | BpmnEvent::TaskScript(_) => (100, 80),

        // Subprocesses and Transactions (expanded)
        BpmnEvent::ActivitySubprocess(_)
        | BpmnEvent::ActivityEventSubprocess(_)
        | BpmnEvent::ActivityTransaction(_) => (350, 200),

        // Data Objects
        BpmnEvent::DataStoreReference(_) => (50, 50),
        BpmnEvent::DataObjectReference(_) => (36, 50),

        // Default case
        _ => (100, 80),
    }
}

fn is_gateway_event(event: &BpmnEvent) -> bool {
    matches!(
        event,
        BpmnEvent::GatewayExclusive
            | BpmnEvent::GatewayInclusive
            | BpmnEvent::GatewayJoin(_)
            | BpmnEvent::GatewayEvent
            | BpmnEvent::GatewayParallel
    )
}

pub fn export_to_xml(bpmn: &String) {
    // Write BPMN to file
    let file_path = "generated_bpmn.bpmn";
    let mut file = File::create(file_path).expect("Unable to create file");
    file.write_all(bpmn.as_bytes())
        .expect("Unable to write data");

    println!("BPMN file generated at: {}", file_path);
}
