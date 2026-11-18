import { useMemo, useRef, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { MindNode } from "../api";

type Placed = { id: string; label: string; x: number; y: number; depth: number; kids: boolean; collapsed: boolean; branch: number };
const W = 190, H = 50, GAP_X = 240, ROW = 64;
const COLORS = ["#2f6fdb", "#e07b22", "#8b4fd1", "#139c8f", "#d64f8b", "#3b9b3b", "#5856d6", "#c0392b"];

function layout(root: MindNode, collapsed: Set<string>) {
  const nodes: Placed[] = []; const edges: { from: Placed; to: Placed }[] = [];
  const leaves = (n: MindNode, id: string): number => collapsed.has(id) || !n.children?.length ? 1 : n.children.reduce((s, c, i) => s + leaves(c, `${id}.${i}`), 0);
  const rootP: Placed = { id: "r", label: root.label, x: 0, y: 0, depth: 0, kids: !!root.children?.length, collapsed: false, branch: -1 };
  nodes.push(rootP);
  const kids = root.children ?? []; const half = Math.ceil(kids.length / 2);
  const side = (list: [MindNode, number][], dir: number) => {
    const total = list.reduce((s, [n, i]) => s + leaves(n, `r.${i}`), 0) * ROW;
    let y = -total / 2;
    const place = (n: MindNode, id: string, depth: number, parent: Placed, branch: number, top: number) => {
      const h = leaves(n, id) * ROW;
      const p: Placed = { id, label: n.label, x: dir * depth * GAP_X, y: top + h / 2, depth, kids: !!n.children?.length, collapsed: collapsed.has(id), branch };
      nodes.push(p); edges.push({ from: parent, to: p });
      if (!collapsed.has(id)) { let t = top; (n.children ?? []).forEach((c, i) => { place(c, `${id}.${i}`, depth + 1, p, branch, t); t += leaves(c, `${id}.${i}`) * ROW; }); }
    };
    list.forEach(([n, i]) => { place(n, `r.${i}`, 1, rootP, i, y); y += leaves(n, `r.${i}`) * ROW; });
  };
  side(kids.slice(0, half).map((n, i) => [n, i] as [MindNode, number]), 1);
  side(kids.slice(half).map((n, i) => [n, i + half] as [MindNode, number]), -1);
  const xs = nodes.map(n => n.x), ys = nodes.map(n => n.y);
  const box = { x: Math.min(...xs) - W / 2 - 30, y: Math.min(...ys) - H / 2 - 30, w: Math.max(...xs) - Math.min(...xs) + W + 60, h: Math.max(...ys) - Math.min(...ys) + H + 60 };
  return { nodes, edges, box };
}

export function MindMap({ root, name }: { root: MindNode; name: string }) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [zoom, setZoom] = useState(0.9); const [pan, setPan] = useState({ x: 0, y: 0 });
  const drag = useRef<{ x: number; y: number; px: number; py: number } | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);
  const { nodes, edges, box } = useMemo(() => layout(root, collapsed), [root, collapsed]);
  const toggle = (id: string) => setCollapsed(s => { const n = new Set(s); n.has(id) ? n.delete(id) : n.add(id); return n; });
  const color = (b: number) => b < 0 ? "#2f6fdb" : COLORS[b % COLORS.length];

  const exportPng = async () => {
    const svg = svgRef.current!; const clone = svg.cloneNode(true) as SVGSVGElement;
    clone.setAttribute("width", String(box.w * 2)); clone.setAttribute("height", String(box.h * 2)); clone.querySelector("g")!.setAttribute("transform", "");
    clone.setAttribute("viewBox", `${box.x} ${box.y} ${box.w} ${box.h}`);
    const img = new Image(); img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(new XMLSerializer().serializeToString(clone));
    await img.decode();
    const c = document.createElement("canvas"); c.width = box.w * 2; c.height = box.h * 2;
    const ctx = c.getContext("2d")!; ctx.fillStyle = "#fff"; ctx.fillRect(0, 0, c.width, c.height); ctx.drawImage(img, 0, 0);
    const b64 = c.toDataURL("image/png").split(",")[1];
    const p = await save({ defaultPath: `${name} - Mind Map.png`, filters: [{ name: "PNG", extensions: ["png"] }] });
    if (p) await invoke("write_base64", { path: p, data: b64 });
  };

  return <div className="mindmap">
    <div className="toolbar">
      <button onClick={() => setCollapsed(new Set())}>Expand All</button>
      <button onClick={() => setCollapsed(new Set(nodes.filter(n => n.depth === 1 && n.kids).map(n => n.id)))}>Collapse All</button>
      <button onClick={() => setZoom(z => Math.max(0.3, z - 0.1))}>−</button><span className="mono">{Math.round(zoom * 100)}%</span><button onClick={() => setZoom(z => Math.min(2.5, z + 0.1))}>＋</button>
      <button onClick={() => { setZoom(0.9); setPan({ x: 0, y: 0 }); }}>Reset</button>
      <div className="grow" /><button onClick={exportPng}>Export PNG</button><button onClick={() => window.print()}>Print / PDF</button>
    </div>
    <svg ref={svgRef} className="mmsvg" viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`}
      onWheel={e => setZoom(z => Math.min(2.5, Math.max(0.3, z - e.deltaY * 0.001)))}
      onMouseDown={e => { drag.current = { x: e.clientX, y: e.clientY, px: pan.x, py: pan.y }; }}
      onMouseMove={e => { if (drag.current) setPan({ x: drag.current.px + (e.clientX - drag.current.x) / zoom, y: drag.current.py + (e.clientY - drag.current.y) / zoom }); }}
      onMouseUp={() => { drag.current = null; }} onMouseLeave={() => { drag.current = null; }}>
      <g transform={`translate(${pan.x} ${pan.y}) scale(${zoom})`} style={{ transformOrigin: "center" }}>
        {edges.map((e, i) => { const mx = (e.from.x + e.to.x) / 2;
          return <path key={i} d={`M${e.from.x},${e.from.y} C${mx},${e.from.y} ${mx},${e.to.y} ${e.to.x},${e.to.y}`} fill="none" stroke={color(e.to.branch)} strokeOpacity={0.55} strokeWidth={2} />; })}
        {nodes.map(n => <g key={n.id} transform={`translate(${n.x - W / 2} ${n.y - H / 2})`} onClick={() => n.kids && n.depth > 0 && toggle(n.id)} style={{ cursor: n.kids ? "pointer" : "default" }}>
          <rect width={W} height={H} rx={12} fill={n.depth === 0 ? color(-1) : color(n.branch)} fillOpacity={n.depth === 0 ? 0.95 : n.depth === 1 ? 0.22 : 0.1} stroke={color(n.branch)} strokeOpacity={0.6} />
          <foreignObject x={6} y={4} width={W - 12} height={H - 8}><div className={`mmlabel d${Math.min(n.depth, 2)}`}>{n.label}{n.kids && n.depth > 0 ? (n.collapsed ? " ⊕" : " ⊖") : ""}</div></foreignObject>
        </g>)}
      </g>
    </svg>
  </div>;
}
