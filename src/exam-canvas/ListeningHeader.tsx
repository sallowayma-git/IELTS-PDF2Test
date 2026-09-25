import { useEffect, useState } from "react";
import { chooseAudioFiles } from "../api/desktopDialogs";
import { bindListeningAudio, managedAudioUrl } from "../api/listeningAudioClient";
import { addAudioEntries, describeIssues, formatDuration } from "../features/import/listeningAudioPlan";
import type { ListeningPartView } from "./listeningWorkspace";

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

// 听力工作区头部：当前 Part 的音频播放器 + 唯一的「添加音频」入口。
// Part 切换已交给底部题号导航（QuestionNavBar），这里只按 selectedPart 展示对应音频。
// Part 视图（含音频绑定）由 ExamCanvas 上提并传入——与底部导航共用同一份数据，
// 添加/替换音频后通过 onAudioChanged 通知画布重新拉取绑定。
export function ListeningHeader({
  itemId,
  mode,
  selectedPart,
  parts,
  onAudioChanged
}: {
  itemId: string;
  mode: "author" | "student";
  selectedPart?: number;
  parts: ListeningPartView[];
  onAudioChanged: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();

  const current = parts.find((part) => part.ordinal === selectedPart) ?? parts[0];
  const noAudio = !parts.some((part) => part.audio);

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
      onAudioChanged();
    }
  };

  const actionLabel = noAudio ? "添加音频" : current.audio ? `替换 ${current.label} 音频` : `为 ${current.label} 添加音频`;

  return <header className="listening-header" data-testid="listening-header">
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
