use crate::common::{bpmn_event::get_node_size, graph::Graph, lane::Lane, node::Node};
use std::collections::{BTreeSet, HashMap};

pub fn assign_xy_to_nodes(graph: &mut Graph) {
    let pool_position_x = 100.0;
    let mut pool_position_y = 100.0;
    let mut node_position_y = 150.0;
    // Wider spacing reduces pile-up and keeps flows readable left-to-right.
    let layer_width = 260.0;
    let lane_x_offset = 30.0;
    let lane_position_x = pool_position_x + lane_x_offset;
    let mut lane_position_y = 100.0;
    let node_x_start = lane_position_x + 50.0;

    {
        let pools = graph.get_pools_mut();
        for pool in pools {
            let mut pool_height = 0.0;
            let mut lane_width = 0.0;
            for lane in pool.get_lanes_mut() {
                lane.sort_nodes_by_layer_id();
                let max_height = find_max_nodes_in_layer(lane.get_layers()) * 140 + 120;
                pool_height += max_height as f64;
                lane.set_height(max_height as f64);
                let new_lane_width = get_lane_width(lane);
                if new_lane_width > lane_width {
                    lane_width = new_lane_width;
                }

                // Important: X must be based on layer_id (graph topology), not on
                // the node vector index. Otherwise branches that share the same layer_id
                // get "squeezed" or drift visually.
                let mut layer_ids: BTreeSet<usize> = BTreeSet::new();
                for n in lane.get_layers().iter() {
                    layer_ids.insert(n.layer_id.unwrap_or(0));
                }

                // Compress sparse layer ids into consecutive columns (prevents huge empty space).
                let layer_to_col: HashMap<usize, usize> = layer_ids
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(col, layer_id)| (layer_id, col))
                    .collect();

                for layer_id in layer_ids {
                    let col = *layer_to_col.get(&layer_id).unwrap_or(&0) as f64;
                    let x = node_x_start + (col * layer_width);
                    let mut y_layer_position = node_position_y;
                    {
                        let nodes_for_this_layer = lane.get_nodes_by_layer_id_mut(layer_id);
                        for node in nodes_for_this_layer {
                            let (node_size_x, node_size_y) =
                                get_node_size(node.event.as_ref().unwrap());
                            let y_offset = if node_size_y < 80 {
                                (80 - node_size_y) as f64 / 2.0
                            } else {
                                0.0
                            };
                            let x_offset = if node_size_x < 100 {
                                (100 - node_size_x) as f64 / 2.0
                            } else {
                                0.0
                            };
                            let old_y = node.y.unwrap_or(y_layer_position);
                            node.set_position(x, old_y, x_offset, y_offset);
                            y_layer_position += 140.0;
                        }
                    }
                }

                node_position_y += max_height as f64;
                lane.set_position(lane_position_x, lane_position_y);
                lane_position_y += max_height as f64;
            }

            if lane_width > pool.width.unwrap_or(0.0) {
                pool.set_width(lane_width + lane_x_offset);
            }
            pool.set_height(pool_height);
            pool.set_position(pool_position_x, pool_position_y);
            pool_position_y += pool_height;
            pool.set_lane_width(lane_width);
        }

        // Enforce a strict left-to-right visual direction: if any sequence flow would point
        // "backwards" (target left of source), shift the target (and everything to its right)
        // to the right until the minimal gap is satisfied.
        enforce_forward_edges(graph, layer_width);
    }
}

fn enforce_forward_edges(graph: &mut Graph, min_gap: f64) {
    // Multiple relaxation passes because shifting one lane can create new conflicts.
    for _ in 0..4 {
        let edges = graph.edges.clone();
        let mut any_changed = false;

        for edge in &edges {
            let (from_pool, from_lane, from_x) = match graph.get_node_by_id(edge.from) {
                Some(n) => (
                    n.pool.clone().unwrap_or_default(),
                    n.lane.clone().unwrap_or_default(),
                    n.x.unwrap_or(0.0),
                ),
                None => continue,
            };
            let (to_pool, to_lane, to_x) = match graph.get_node_by_id(edge.to) {
                Some(n) => (
                    n.pool.clone().unwrap_or_default(),
                    n.lane.clone().unwrap_or_default(),
                    n.x.unwrap_or(0.0),
                ),
                None => continue,
            };

            if to_x + 1.0 < from_x + min_gap {
                let dx = (from_x + min_gap) - to_x;
                shift_lane_from_x(graph, &to_pool, &to_lane, to_x, dx);
                any_changed = true;
            }

            // If edge crosses lanes, also ensure the source isn't shifted behind its own lane's
            // later nodes (keeps lanes visually consistent).
            if from_pool != to_pool || from_lane != to_lane {
                shift_lane_from_x(graph, &from_pool, &from_lane, from_x, 0.0);
            }
        }

        if !any_changed {
            break;
        }
    }
}

fn shift_lane_from_x(graph: &mut Graph, pool_name: &str, lane_name: &str, threshold_x: f64, dx: f64) {
    if dx.abs() < f64::EPSILON {
        return;
    }

    for pool in graph.get_pools_mut() {
        if pool.get_pool_name() != pool_name {
            continue;
        }
        for lane in pool.get_lanes_mut() {
            if lane.get_lane() != lane_name {
                continue;
            }
            for node in lane.get_layers_mut() {
                let x = node.x.unwrap_or(0.0);
                if x + 0.5 >= threshold_x {
                    let y = node.y.unwrap_or(0.0);
                    let xo = node.x_offset.unwrap_or(0.0);
                    let yo = node.y_offset.unwrap_or(0.0);
                    node.set_position(x + dx, y, xo, yo);
                }
            }
        }
    }
}

fn find_max_nodes_in_layer(nodes: &Vec<Node>) -> usize {
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for n in nodes {
        *counts.entry(n.layer_id.unwrap_or(0)).or_insert(0) += 1;
    }
    counts.values().copied().max().unwrap_or(1)
}

fn get_lane_width(lane: &Lane) -> f64 {
    let last_node = lane.get_layers().last().unwrap();
    let last_layer = last_node.layer_id.unwrap_or(0);
    if last_layer == 0 || last_layer == 1 {
        return 350.0;
    } else {
        return (last_layer) as f64 * 260.0 + 450.0;
    }
}
