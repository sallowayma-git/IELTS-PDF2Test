import { Plus, X, RotateCcw } from "lucide-react";
import type { AuthoringPatchV2, ContentNodeV2, IeltsAuthoringIRV2 } from "../../types";
import { locateContentNode } from "../../services/authoringV2Patches";

type Rect = [number, number, number, number];
function RectFields({ value, onChange }: { value: Rect; onChange: (value: Rect) => void }) {
  return <div className="crop-grid">{value.map((number, index) => <label key={index}>
    {["横向位置", "纵向位置", "宽度", "高度"][index]}
    <input type="number" min={0} max={1} step={0.01} value={number} onChange={(event) => {
      const next = [...value] as Rect;
      next[index] = Number(event.target.value);
      next[0] = Math.min(next[0], 0.99); next[1] = Math.min(next[1], 0.99);
      next[2] = Math.max(0.01, Math.min(next[2], 1 - next[0]));
      next[3] = Math.max(0.01, Math.min(next[3], 1 - next[1]));
      onChange(next);
    }} />
  </label>)}</div>;
}

export function SelectionInspector({ draft, selectedId, onPatch, onClose }: {
  draft: IeltsAuthoringIRV2; selectedId?: string; onPatch: (patch: AuthoringPatchV2) => void; onClose: () => void;
}) {
  const node = selectedId ? locateContentNode(draft, selectedId)?.node : undefined;
  if (!node || !["figure", "diagram", "image", "table_cell"].includes(node.type)) return null;
  const media = node.type === "figure" || node.type === "diagram" || node.type === "image" ? node : undefined;
  const hotspots = media && media.type !== "image" ? media.hotspots ?? [] : [];
  return <aside className="workspace-selection" aria-label="选中内容设置">
    <header><strong>{media ? "图片与答案位置" : "单元格"}</strong>
      <button className="ghost small" title="关闭设置" aria-label="关闭设置" onClick={onClose}><X size={16} /></button></header>
    {media ? <>
      <label>图片<select value={media.assetId} onChange={(event) => onPatch({ op: "setNodeAttrs", nodeId: media.id, attrs: { assetId: event.target.value } })}>
        {draft.assets.filter((asset) => asset.mime.startsWith("image/")).map((asset) => <option key={asset.assetId} value={asset.assetId}>{asset.altText || asset.assetId}</option>)}
      </select></label>
      <details open><summary>裁剪</summary><RectFields value={media.crop ?? [0, 0, 1, 1]} onChange={(crop) => onPatch({ op: "cropAsset", nodeId: media.id, crop })} />
        <button className="ghost small" title="恢复完整图片" aria-label="恢复完整图片" onClick={() => onPatch({ op: "cropAsset", nodeId: media.id, crop: null })}><RotateCcw size={16} /></button>
      </details>
      {media.type !== "image" ? <>
        {hotspots.map((hotspot) => <fieldset key={hotspot.hotspotId}>
          <legend>答案 {draft.answerSlots[hotspot.slotId]?.displayLabel}</legend>
          <select aria-label="对应答案" value={hotspot.slotId} onChange={(event) => onPatch({ op: "setHotspot", nodeId: media.id, hotspot: { ...hotspot, slotId: event.target.value } })}>
            {Object.values(draft.answerSlots).filter((slot) => slot.hostNodeId === media.id).map((slot) => <option key={slot.slotId} value={slot.slotId}>{slot.displayLabel}</option>)}
          </select>
          <RectFields value={hotspot.normalizedRect} onChange={(normalizedRect) => onPatch({ op: "setHotspot", nodeId: media.id, hotspot: { ...hotspot, normalizedRect } })} />
          <button className="ghost small" aria-label="删除答案位置" title="删除答案位置" onClick={() => onPatch({ op: "removeHotspot", nodeId: media.id, hotspotId: hotspot.hotspotId })}><X size={16} /></button>
        </fieldset>)}
        <button className="ghost small" onClick={() => {
          const slot = Object.values(draft.answerSlots).find((slot) => slot.hostNodeId === media.id && !hotspots.some((hotspot) => hotspot.slotId === slot.slotId));
          if (slot) onPatch({ op: "setHotspot", nodeId: media.id, hotspot: { hotspotId: crypto.randomUUID(), slotId: slot.slotId, normalizedRect: [0.1, 0.1, 0.2, 0.1] } });
        }} disabled={!Object.values(draft.answerSlots).some((slot) => slot.hostNodeId === media.id && !hotspots.some((hotspot) => hotspot.slotId === slot.slotId))}><Plus size={16} />答案位置</button>
      </> : null}
    </> : null}
    {node.type === "table_cell" ? <>
      <label>跨行<input type="number" min={1} max={20} value={node.rowSpan} onChange={(event) => onPatch({ op: "setNodeAttrs", nodeId: node.id, attrs: { rowSpan: Math.max(1, Number(event.target.value)) } })} /></label>
      <label>跨列<input type="number" min={1} max={20} value={node.colSpan} onChange={(event) => onPatch({ op: "setNodeAttrs", nodeId: node.id, attrs: { colSpan: Math.max(1, Number(event.target.value)) } })} /></label>
      <label>表头<select value={node.headerScope} onChange={(event) => onPatch({ op: "setNodeAttrs", nodeId: node.id, attrs: { headerScope: event.target.value as Extract<ContentNodeV2, { type: "table_cell" }>["headerScope"] } })}>
        <option value="none">无</option><option value="row">行表头</option><option value="column">列表头</option>
      </select></label>
    </> : null}
  </aside>;
}
