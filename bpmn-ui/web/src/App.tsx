import React, { useEffect, useMemo, useRef, useState } from "react";
import BpmnModeler from "bpmn-js/lib/Modeler";

const DEFAULT_DSL = ``;

const EMPTY_BPMN = `<?xml version="1.0" encoding="UTF-8"?>
<bpmn:definitions xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  xmlns:bpmn="http://www.omg.org/spec/BPMN/20100524/MODEL"
  xmlns:bpmndi="http://www.omg.org/spec/BPMN/20100524/DI"
  xmlns:dc="http://www.omg.org/spec/DD/20100524/DC"
  xmlns:di="http://www.omg.org/spec/DD/20100524/DI"
  id="Definitions_1"
  targetNamespace="http://bpmn.io/schema/bpmn">
  <bpmn:process id="Process_1" isExecutable="false">
    <bpmn:startEvent id="StartEvent_1" />
  </bpmn:process>
  <bpmndi:BPMNDiagram id="BPMNDiagram_1">
    <bpmndi:BPMNPlane id="BPMNPlane_1" bpmnElement="Process_1" />
  </bpmndi:BPMNDiagram>
</bpmn:definitions>`;

async function generateFromServer(dsl: string): Promise<string> {
  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), 30000);

  const res = await fetch("/api/generate", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ dsl }),
    signal: controller.signal,
  });
  window.clearTimeout(timeout);
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error(data?.error || `HTTP ${res.status}`);
  }
  if (typeof data?.xml !== "string") throw new Error("Bad server response");
  return data.xml;
}

async function convertToDsl(text: string): Promise<string> {
  const res = await fetch("/api/convert", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ text }),
  });
  const data = await res.json().catch(() => ({}));
  if (!res.ok) {
    throw new Error(data?.error || `HTTP ${res.status}`);
  }
  if (typeof data?.dsl !== "string") throw new Error("Bad server response");
  return data.dsl;
}

function downloadText(filename: string, text: string, mime = "application/xml") {
  const blob = new Blob([text], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

export function App() {
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const modelerRef = useRef<BpmnModeler | null>(null);

  const [dsl, setDsl] = useState(DEFAULT_DSL);
  const [xml, setXml] = useState("");
  const [status, setStatus] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [diagramTheme, setDiagramTheme] = useState<string>(() => {
    try {
      return window.localStorage.getItem("diagramTheme") || "default";
    } catch {
      return "default";
    }
  });

  const modeler = useMemo(() => new BpmnModeler({ keyboard: { bindTo: document } }), []);

  useEffect(() => {
    modelerRef.current = modeler;
    if (!canvasRef.current) return;
    modeler.attachTo(canvasRef.current);
    modeler.importXML(EMPTY_BPMN).catch(() => {});
    try {
      const canvas = modeler.get("canvas");
      canvas.resized();
    } catch {
      // ignore
    }
    return () => {
      modeler.destroy();
      modelerRef.current = null;
    };
  }, [modeler]);

  async function loadXmlIntoModeler(nextXml: string) {
    const result: any = await modeler.importXML(nextXml);
    const canvas = modeler.get("canvas");
    const elementRegistry = modeler.get("elementRegistry");

    // Ensure we render the top-level diagram root (Collaboration/Process) explicitly.
    try {
      const all = elementRegistry.getAll();
      const collaboration = all.find(
        (e: any) => e?.businessObject?.$type === "bpmn:Collaboration",
      );
      const process = all.find((e: any) => e?.businessObject?.$type === "bpmn:Process");
      if (collaboration) {
        canvas.setRootElement(collaboration);
      } else if (process) {
        canvas.setRootElement(process);
      }
    } catch {
      // ignore
    }
    // Let the browser compute container size before fitting.
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
    canvas.resized();

    // Robust fit: compute bbox of all elements and set viewbox first.
    try {
      const els = elementRegistry
        .getAll()
        .filter((e: any) => e && e.x != null && e.y != null && e.width != null && e.height != null);
      if (els.length) {
        let minX = Infinity,
          minY = Infinity,
          maxX = -Infinity,
          maxY = -Infinity;
        for (const e of els) {
          minX = Math.min(minX, e.x);
          minY = Math.min(minY, e.y);
          maxX = Math.max(maxX, e.x + e.width);
          maxY = Math.max(maxY, e.y + e.height);
        }
        if (isFinite(minX) && isFinite(minY) && isFinite(maxX) && isFinite(maxY)) {
          const pad = 80;
          canvas.viewbox({
            x: minX - pad,
            y: minY - pad,
            width: maxX - minX + pad * 2,
            height: maxY - minY + pad * 2,
          });
        }
      }
    } catch {
      // ignore
    }

    canvas.zoom("fit-viewport");

    // Return some debug info to show in UI status.
    return {
      warnings: Array.isArray(result?.warnings) ? result.warnings.length : 0,
      warningPreview: Array.isArray(result?.warnings)
        ? result.warnings
            .slice(0, 3)
            .map((w: any) => (w?.message ? String(w.message) : String(w)))
            .join(" | ")
        : "",
      elements: (() => {
        try {
          return elementRegistry.getAll().length;
        } catch {
          return 0;
        }
      })(),
    };
  }

  async function onGenerate() {
    setBusy(true);
    setStatus("Generating...");
    try {
      const nextXml = await generateFromServer(dsl);
      setXml(nextXml);
      try {
        const info: any = await loadXmlIntoModeler(nextXml);
        const w = info?.warnings ?? 0;
        const n = info?.elements ?? 0;
        const wp = info?.warningPreview ? ` First: ${info.warningPreview}` : "";
        setStatus(
          `Generated and loaded (${nextXml.length} chars, ${n} elements, ${w} warnings).${wp}`,
        );
      } catch (e) {
        setStatus(`Generated XML but failed to load into modeler: ${String((e as Error)?.message || e)}`);
      }
    } catch (e) {
      setStatus(String((e as Error)?.message || e));
    } finally {
      setBusy(false);
    }
  }

  async function onConvert() {
    setBusy(true);
    setStatus("Converting to DSL...");
    try {
      const nextDsl = await convertToDsl(dsl);
      setDsl(nextDsl);
      setStatus("Converted. You can Generate now.");
    } catch (e) {
      setStatus(String((e as Error)?.message || e));
    } finally {
      setBusy(false);
    }
  }

  async function onExportXml() {
    setStatus("Exporting XML...");
    try {
      const { xml: saved } = await modeler.saveXML({ format: true });
      downloadText("diagram.bpmn", saved);
      setStatus("Downloaded diagram.bpmn");
    } catch (e) {
      setStatus(String((e as Error)?.message || e));
    }
  }

  async function onApplyXml() {
    if (!xml.trim()) {
      setStatus("XML is empty");
      return;
    }
    setBusy(true);
    setStatus("Loading XML...");
    try {
      await loadXmlIntoModeler(xml);
      setStatus("Loaded XML into editor.");
    } catch (e) {
      setStatus(String((e as Error)?.message || e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="layout">
      <header className="header">
        <div className="title">BPMN UI</div>
        <div className="actions">
          <select
            aria-label="Diagram theme"
            value={diagramTheme}
            onChange={(e) => {
              const next = e.target.value;
              setDiagramTheme(next);
              try {
                window.localStorage.setItem("diagramTheme", next);
              } catch {
                // ignore
              }
            }}
            disabled={busy}
            className="themeSelect"
          >
            <option value="default">Theme: Default</option>
            <option value="ocean">Theme: Ocean</option>
            <option value="purple">Theme: Purple</option>
            <option value="mono">Theme: Mono</option>
          </select>
          <button disabled={busy} onClick={onConvert}>
            Convert text → DSL
          </button>
          <button disabled={busy} onClick={onGenerate}>
            Generate from DSL
          </button>
          <button disabled={busy} onClick={onApplyXml}>
            Load XML
          </button>
          <button disabled={busy} onClick={onExportXml}>
            Download .bpmn
          </button>
        </div>
      </header>

      <div className="content">
        <section className="panel">
          <div className="panelTitle">Process DSL</div>
          <textarea
            value={dsl}
            onChange={(e) => setDsl(e.target.value)}
            placeholder={`= Process\n== Main\n# Start\n- [API] GET /...\n. End`}
          />
          <div className="panelTitle">BPMN XML (optional)</div>
          <textarea value={xml} onChange={(e) => setXml(e.target.value)} />
          <div className="status">{status}</div>
          <div className="hint">
            Web: <code>{typeof window !== "undefined" ? window.location.origin : ""}</code> • API:{" "}
            <code>/api</code>
          </div>
        </section>

        <section className={`modelerWrap diagramTheme-${diagramTheme}`}>
          <div className="modeler" ref={canvasRef} />
        </section>
      </div>
    </div>
  );
}

