import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";
import { chooseAudioFiles, chooseAudioFolder, listenForFileDrops } from "../../api/desktopDialogs";
import { listListeningAudioFolder, probeListeningAudioFiles } from "../../api/listeningAudioClient";
import {
  addAudioEntries,
  addAudioLaterDecision,
  assignmentNotices,
  describeIssues,
  formatDuration,
  listeningDecision,
  moveEntry,
  probeSummaryFrom,
  readingDecision,
  removeEntry,
  withProbe,
  type AudioEntry,
  type ListeningImportDecision
} from "./listeningAudioPlan";

// 识别到听力试卷后弹出：确认听力并提供音频（拖入 MP3、选文件、选文件夹），
// 或改回阅读。音频顺序即 Part 顺序，可上下调整；每个文件显示检查结果。
// 「稍后添加音频」也可以：条目照常建立，但在补齐音频之前不算音频就绪。
export function ListeningAudioDialog({
  paperName,
  cues,
  onDecide,
  onCancel
}: {
  paperName: string;
  cues: string[];
  onDecide: (decision: ListeningImportDecision) => void;
  onCancel: () => void;
}) {
  const [entries, setEntries] = useState<AudioEntry[]>([]);
  const [hovering, setHovering] = useState(false);
  const [notice, setNotice] = useState<string>();
  const alive = useRef(true);

  const entriesRef = useRef<AudioEntry[]>([]);
  const commit = useCallback((next: AudioEntry[]) => {
    entriesRef.current = next;
    setEntries(next);
  }, []);

  const addPaths = useCallback((incoming: Array<string | AudioEntry>) => {
    const current = entriesRef.current;
    const { entries: next, ignored } = addAudioEntries(current, incoming);
    const known = new Set(current.map((entry) => entry.path));
    const added = next.filter((entry) => !known.has(entry.path));
    commit(next);
    setNotice(ignored.length ? `已忽略非音频文件：${ignored.join("、")}` : undefined);
    if (!added.length) return;
    probeListeningAudioFiles(added.map((entry) => entry.path))
      .then((results) => {
        if (!alive.current) return;
        commit(
          added.reduce((acc, entry, index) => {
            const summary = probeSummaryFrom(results[index]);
            return summary ? withProbe(acc, entry.path, summary) : acc;
          }, entriesRef.current)
        );
      })
      .catch(() => undefined);
  }, [commit]);
  useEffect(() => {
    alive.current = true;
    let unlisten: (() => void) | undefined;
    listenForFileDrops({ onDrop: (paths) => addPaths(paths), onHover: setHovering })
      .then((stop) => {
        if (alive.current) unlisten = stop;
        else stop();
      })
      .catch(() => undefined);
    return () => {
      alive.current = false;
      unlisten?.();
    };
  }, [addPaths]);

  const pickFolder = async () => {
    const folder = await chooseAudioFolder();
    if (!folder) return;
    const files = await listListeningAudioFolder(folder).catch(() => []);
    if (!files.length) {
      setNotice("该文件夹里没有 MP3 文件。");
      return;
    }
    addPaths(files.map((file) => ({ path: file.path, name: file.name, sizeBytes: file.sizeBytes })));
  };

  // 浏览器开发预览：没有真实路径，只按文件名登记（桌面端由 Tauri 拖放事件提供路径）。
  const onHtmlDrop = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault();
    setHovering(false);
    const names = Array.from(event.dataTransfer?.files ?? []).map((file) => file.name);
    if (names.length) addPaths(names);
  };

  const notices = assignmentNotices(entries);

  return (
    <div className="drawer-scrim listening-dialog-scrim" role="presentation" onClick={(event) => event.stopPropagation()}>
      <aside
        className="drawer drawer-wide listening-audio-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="确认听力试卷并添加音频"
        data-testid="listening-audio-dialog"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="drawer-head">
          <h2>这是一份听力试卷吗？</h2>
          <button className="ghost small" onClick={onCancel} aria-label="关闭">×</button>
        </header>

        <div className="drawer-body">
          <p className="drawer-hint">
            <strong className="file-name">{paperName}</strong> 看起来是雅思听力（{cues.length ? `${cues.length} 处听力特征` : "听力特征"}）。
            请提供每个 Part 的 MP3 音频，音频会复制到应用内保存，之后移动或删除原文件也不影响。
          </p>

          <div
            className={`audio-drop-zone${hovering ? " is-hovering" : ""}`}
            data-testid="listening-audio-drop"
            onDragOver={(event) => {
              event.preventDefault();
              setHovering(true);
            }}
            onDragLeave={() => setHovering(false)}
            onDrop={onHtmlDrop}
          >
            <p>把 MP3 拖到这里</p>
            <div className="button-row">
              <button className="ghost" data-testid="listening-audio-pick-files" onClick={() => chooseAudioFiles().then(addPaths)}>
                选择音频文件
              </button>
              <button className="ghost" data-testid="listening-audio-pick-folder" onClick={pickFolder}>
                选择文件夹
              </button>
            </div>
          </div>

          {entries.length ? (
            <ol className="picked-file-list audio-part-list" data-testid="listening-audio-parts">
              {entries.map((entry, index) => (
                <li key={entry.path} data-part={index + 1}>
                  <span className="audio-part-label">Part {index + 1}</span>
                  <span className="file-name">{entry.name}</span>
                  <span className={`audio-probe audio-probe-${entry.probe?.status ?? "pending"}`}>
                    {entry.probe
                      ? entry.probe.status === "passed"
                        ? formatDuration(entry.probe.durationMs) || "可播放"
                        : describeIssues(entry.probe.issueCodes) || "无法通过检查"
                      : "检查中…"}
                  </span>
                  <span className="audio-part-actions">
                    <button className="ghost small" aria-label={`上移 ${entry.name}`} disabled={index === 0}
                      onClick={() => commit(moveEntry(entriesRef.current, index, -1))}>↑</button>
                    <button className="ghost small" aria-label={`下移 ${entry.name}`} disabled={index === entries.length - 1}
                      onClick={() => commit(moveEntry(entriesRef.current, index, 1))}>↓</button>
                    <button className="ghost small" aria-label={`移除 ${entry.name}`}
                      onClick={() => commit(removeEntry(entriesRef.current, index))}>移除</button>
                  </span>
                </li>
              ))}
            </ol>
          ) : (
            <p className="empty compact">尚未添加音频。</p>
          )}

          {notices.map((item) => (
            <p key={item.kind} className={item.kind === "blocked" ? "error-text" : "drawer-hint"}>{item.message}</p>
          ))}
          {notice ? <p className="drawer-hint">{notice}</p> : null}
        </div>

        <footer className="drawer-foot">
          <button className="ghost" data-testid="listening-switch-reading" onClick={() => onDecide(readingDecision())}>
            这是阅读题
          </button>
          <button className="ghost" data-testid="listening-audio-later" onClick={() => onDecide(addAudioLaterDecision())}>
            稍后添加音频
          </button>
          <button
            className="primary"
            data-testid="listening-confirm"
            disabled={!entries.length}
            onClick={() => onDecide(listeningDecision(entries))}
          >
            确认听力并导入
          </button>
        </footer>
      </aside>
    </div>
  );
}
