import { useCallback, useEffect, useState } from "react";
import { chooseAudioFiles } from "../api/desktopDialogs";
import {
  bindListeningAudio,
  getListeningAudio,
  managedAudioUrl,
  type ListeningAudioStatus
} from "../api/listeningAudioClient";
import type { IeltsAuthoringIRV2 } from "../types";
import { addAudioEntries, describeIssues, formatDuration } from "../features/import/listeningAudioPlan";
import { listeningParts, type ListeningPartView } from "./listeningWorkspace";

function AudioPlayer({ part }: { part: ListeningPartView }) {
  const [src, setSrc] = useState<string>();
  const path = part.audio?.playable ? part.audio.managedPath : undefined;
  useEffect(() => {
    let alive = true;
    setSrc(undefined);
    if (path) managedAudioUrl(path).then((url) => alive && setSrc(url)).catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [path]);
  if (!part.audio) return <span className="listening-audio-state is-missing">{part.label} 还没有音频</span>;
  if (!part.audio.playable) {
    return <span className="listening-audio-state is-blocked" role="alert">
      {part.label} 音频无法使用：{describeIssues(part.audio.issueCodes) || "检查未通过"}
    </span>;
  }
  return <span className="listening-audio-player">
    {src ? <audio controls preload="metadata" src={src} data-testid={`listening-audio-${part.ordinal}`} /> : null}
    <small>{part.audio.originalName} · {formatDuration(part.audio.durationMs ?? undefined)}</small>
  </span>;
}

// 听力工作区头部：Part 导航 + 当前 Part 的音频播放器 + 唯一的「添加音频」入口。
export function ListeningHeader({
  itemId,
  authoring,
  mode,
  selectedPart,
  onSelectPart
}: {
  itemId: string;
  authoring: IeltsAuthoringIRV2;
  mode: "author" | "student";
  selectedPart?: number;
  onSelectPart: (ordinal: number) => void;
}) {
  const [status, setStatus] = useState<ListeningAudioStatus>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const reload = useCallback((verify: boolean) => {
    getListeningAudio(itemId, verify).then(setStatus).catch(() => setStatus(undefined));
  }, [itemId]);
  useEffect(() => reload(true), [reload]);

  const parts = listeningParts(authoring, status?.bindings ?? []);
  const current = parts.find((part) => part.ordinal === selectedPart) ?? parts[0];
  const noAudio = !status?.bindings.length;

  // 没有任何音频：选中的文件按自然顺序落到 Part 1..n；否则替换/补齐当前 Part。
  const addAudio = async () => {
    const picked = await chooseAudioFiles();
    if (!picked.length) return;
    setBusy(true);
    setError(undefined);
    try {
      if (noAudio) {
        const ordered = addAudioEntries([], picked).entries;
        for (const [index, entry] of ordered.entries()) await bindListeningAudio(itemId, index + 1, entry.path);
      } else {
        await bindListeningAudio(itemId, current.ordinal, picked[0]);
      }
    } catch {
      setError("音频没有保存成功，请换一个文件再试。");
    } finally {
      setBusy(false);
      reload(false);
    }
  };

  const actionLabel = noAudio ? "添加音频" : current.audio ? `替换 ${current.label} 音频` : `为 ${current.label} 添加音频`;

  return <header className="listening-header" data-testid="listening-header">
    <nav className="listening-part-nav" aria-label="听力 Part">
      {parts.map((part) => <button
        key={part.ordinal}
        type="button"
        className={`listening-part-tab${part.ordinal === current.ordinal ? " active" : ""}${part.audio && !part.audio.playable ? " is-blocked" : ""}`}
        aria-pressed={part.ordinal === current.ordinal}
        onClick={() => onSelectPart(part.ordinal)}
      >{part.label}{part.audio ? "" : " ·"}</button>)}
    </nav>
    <div className="listening-audio-row">
      <AudioPlayer part={current} />
      {mode === "author" ? (
        <button type="button" className={noAudio ? "primary small" : "ghost small"} data-testid="listening-add-audio" disabled={busy} onClick={addAudio}>
          {busy ? "正在保存音频…" : actionLabel}
        </button>
      ) : null}
    </div>
    {error ? <p className="error-text">{error}</p> : null}
  </header>;
}
