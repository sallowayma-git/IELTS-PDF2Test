import { useEffect, useState } from "react";
import { listLlmProfiles } from "../../api/tauriCommands";
import { go } from "../../app/router";
import { hasCloudConnection } from "../settings/settingsLogic";
import { choosePdfFolderSources, chooseSourceFiles, type PickedPath } from "../../api/desktopDialogs";
import { detectImportModality } from "../../api/listeningAudioClient";
import { formatPageRanges, planImportSplit, SPLIT_CONFIDENCE_LABEL, SPLIT_SIGNAL_LABEL, type ImportSplitPreview } from "../../api/processingClient";
import type { ImportRejection } from "./useImportFiles";
import { ListeningAudioDialog } from "./ListeningAudioDialog";
import { listeningCandidates, type ListeningCandidate, type ListeningImportDecision } from "./listeningAudioPlan";

// 导入不再是独立页面，而是题库页上的抽屉（计划 §3.1 / §12.1）。
// 用户只选文件；title 取文件名、modality 自动、parseMode 自动、云端沿用设置页开关。
// 已删除的导入前必填项：category、frequency、tags、parseMode、per-import 云端勾选。
export function ImportDrawer({
  open,
  busy,
  stageMessage,
  error,
  rejected,
  onClose,
  onImport
}: {
  open: boolean;
  busy: boolean;
  stageMessage?: string;
  error?: string;
  rejected: ImportRejection[];
  onClose: () => void;
  onImport: (files: PickedPath[], decisions?: Record<string, ListeningImportDecision>) => void;
}) {
  const [files, setFiles] = useState<PickedPath[]>([]);
  const [checking, setChecking] = useState(false);
  // 识别为听力的文件逐个确认；decisions 以源文件路径为键。
  const [queue, setQueue] = useState<ListeningCandidate<PickedPath>[]>([]);
  const [decisions, setDecisions] = useState<Record<string, ListeningImportDecision>>({});
  // 整套卷子：一份 PDF 里识别到多篇时先让用户确认拆分（默认拆），每篇成为题库里独立的一行。
  const [splitPlans, setSplitPlans] = useState<ImportSplitPreview[]>([]);
  const [keepWhole, setKeepWhole] = useState<Set<string>>(new Set());
  const [pendingCandidates, setPendingCandidates] = useState<ListeningCandidate<PickedPath>[]>([]);
  // 真正送去导入的文件（已带上拆分选择）；听力逐个确认结束后用它。
  const [prepared, setPrepared] = useState<PickedPath[]>([]);
  // `undefined` = 还没查到；查不到时不提示（不猜）。
  const [cloudConnected, setCloudConnected] = useState<boolean | undefined>();
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    listLlmProfiles()
      .then((profiles) => { if (!cancelled) setCloudConnected(hasCloudConnection(profiles)); })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [open]);

  if (!open) return null;

  const addFiles = (picked: PickedPath[]) => {
    if (!picked.length) return;
    // 文件变了，拆分预览要重新检查。
    setSplitPlans([]);
    setFiles((current) => {
      const byPath = new Map(current.map((file) => [file.path, file]));
      for (const file of picked) byPath.set(file.path, file);
      return [...byPath.values()];
    });
  };

  const remove = (path: string) => {
    setFiles((current) => current.filter((file) => file.path !== path));
    setSplitPlans((current) => current.filter((plan) => plan.path !== path));
  };

  const proceed = (ready: PickedPath[], candidates: ListeningCandidate<PickedPath>[]) => {
    setPrepared(ready);
    if (!candidates.length) {
      onImport(ready);
      return;
    }
    setDecisions({});
    setQueue(candidates);
  };

  const close = () => {
    setFiles([]);
    setQueue([]);
    setDecisions({});
    setSplitPlans([]);
    setKeepWhole(new Set());
    setPendingCandidates([]);
    setPrepared([]);
    onClose();
  };

  const start = async () => {
    if (splitPlans.length) {
      // 第二次点击 = 确认拆分选择。
      const splitPaths = new Set(splitPlans.filter((plan) => !keepWhole.has(plan.path)).map((plan) => plan.path));
      const ready = files.map((file) => (splitPaths.has(file.path) ? { ...file, splitPassages: true } : file));
      setSplitPlans([]);
      proceed(ready, pendingCandidates);
      return;
    }
    setChecking(true);
    const detections = await detectImportModality(files.map((file) => file.path)).catch(() => []);
    const candidates = listeningCandidates(files, detections);
    // 听力条目本身就是一整套 Part 1–4（音频按 Part 绑定），只拆阅读。
    const listeningPaths = new Set(candidates.map((candidate) => candidate.file.path));
    const readingPdfs = files.filter((file) => !listeningPaths.has(file.path) && /\.pdf$/i.test(file.name));
    const plans = (await planImportSplit(readingPdfs.map((file) => file.path)).catch(() => []))
      .filter((plan) => plan.segments.length >= 2);
    setChecking(false);
    if (plans.length) {
      setPendingCandidates(candidates);
      // 信号不够一致的文件默认不拆，用户核对后可以勾上。
      setKeepWhole(new Set(plans.filter((plan) => !plan.defaultSplit).map((plan) => plan.path)));
      setSplitPlans(plans);
      return;
    }
    proceed(files, candidates);
  };

  const decide = (decision: ListeningImportDecision) => {
    const [current, ...rest] = queue;
    if (!current) return;
    const next = { ...decisions, [current.file.path]: decision };
    setDecisions(next);
    setQueue(rest);
    if (!rest.length) onImport(prepared, next);
  };

  const pending = queue[0];

  return (
    <div className="drawer-scrim" role="presentation" onClick={busy ? undefined : close}>
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-label="导入 PDF 或 DOCX"
        data-testid="import-drawer"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="drawer-head">
          <h2>导入题目</h2>
          <button className="ghost small" onClick={close} disabled={busy} aria-label="关闭">×</button>
        </header>

        <div className="drawer-body">
          <p className="drawer-hint">选择一份或多份 PDF / DOCX / TXT / MD。标题默认取文件名，之后可以在题目里直接改。</p>
          {cloudConnected === false ? (
            <p className="drawer-hint" data-testid="import-cloud-offline">
              未连接云端，仅本地识别 ·{" "}
              <button className="ghost small" onClick={() => { close(); go("/settings"); }}>去连接</button>
            </p>
          ) : null}
          <div className="button-row">
            <button className="ghost" data-testid="import-pick-files" disabled={busy} onClick={() => chooseSourceFiles().then(addFiles)}>
              选择文件
            </button>
            <button className="ghost" data-testid="import-pick-folder" disabled={busy} onClick={() => choosePdfFolderSources().then(addFiles)}>
              选择 PDF 文件夹
            </button>
          </div>

          {files.length ? (
            <ul className="picked-file-list" data-testid="import-picked-files">
              {files.map((file) => (
                <li key={file.path}>
                  <span className="file-name">{file.name}</span>
                  <button className="ghost small" onClick={() => remove(file.path)} disabled={busy} aria-label={`移除 ${file.name}`}>
                    移除
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="empty compact">尚未选择文件。</p>
          )}

          {splitPlans.length ? (
            <section className="split-plan-panel" data-testid="import-split-plan">
              <p className="drawer-hint">
                下面的文件里有整套卷子。拆分后每篇是题库里独立的一行，分别识别、编辑和发布；在题库里切换篇章即可。
              </p>
              {splitPlans.map((plan) => {
                const name = files.find((file) => file.path === plan.path)?.name ?? plan.path;
                const split = !keepWhole.has(plan.path);
                return (
                  <div key={plan.path} className="split-plan-file" data-testid="import-split-file">
                    <label className="split-plan-toggle">
                      <input
                        type="checkbox"
                        checked={split}
                        disabled={busy}
                        data-testid="import-split-toggle"
                        onChange={(event) => setKeepWhole((current) => {
                          const next = new Set(current);
                          if (event.target.checked) next.delete(plan.path); else next.add(plan.path);
                          return next;
                        })}
                      />
                      <span>
                        <strong className="file-name">{name}</strong>
                        {split ? ` · 识别到 ${plan.segments.length} 篇，拆成 ${plan.segments.length} 条` : ` · 识别到 ${plan.segments.length} 篇 · 不拆分，整份作为一条导入`}
                      </span>
                    </label>
                    {!plan.defaultSplit ? (
                      <p className="drawer-hint split-plan-warning" data-testid="import-split-weak">
                        分篇信号不够一致，默认整份导入；核对下面的分篇后可以勾选拆分。
                      </p>
                    ) : null}
                    <ul className="split-plan-segments">
                      {plan.segments.map((segment) => (
                        <li key={`${segment.label}-${segment.pages[0]}`} data-testid="import-split-segment">
                          <div className="split-plan-segment-head">
                            <strong>{segment.label}</strong>
                            <span>第 {formatPageRanges(segment.pages)} 页</span>
                            {segment.questionRange ? <span>题 {segment.questionRange[0]}–{segment.questionRange[1]}</span> : null}
                            <span className={`split-confidence ${segment.confidence}`}>{SPLIT_CONFIDENCE_LABEL[segment.confidence]}</span>
                          </div>
                          <div className="split-plan-segment-meta">
                            <span>依据：{segment.signals.map((signal) => SPLIT_SIGNAL_LABEL[signal] ?? signal).join("、") || "无"}</span>
                            <span>
                              {segment.answers.length
                                ? `答案 ${segment.answers.length} 条（已从答案页按题号分出）`
                                : segment.fallbackAnswerPages.length
                                  ? `答案未能分出，附第 ${formatPageRanges(segment.fallbackAnswerPages)} 页答案页`
                                  : "未找到答案"}
                            </span>
                          </div>
                        </li>
                      ))}
                    </ul>
                    {plan.warnings.map((warning) => (
                      <p key={warning} className="drawer-hint split-plan-warning">{warning}</p>
                    ))}
                    {split && plan.unassignedPages.length ? (
                      <p className="drawer-hint">第 {formatPageRanges(plan.unassignedPages)} 页不属于任何一篇（封面、说明、听力等），不会导入。</p>
                    ) : null}
                  </div>
                );
              })}
            </section>
          ) : null}

          {stageMessage ? (
            <div className="progress-panel" data-testid="import-stage">
              <div className="spinner" aria-hidden="true" />
              <div><strong>{stageMessage}</strong><p>建立条目后会立即返回题库，识别在后台继续。</p></div>
            </div>
          ) : null}

          {error ? <p className="error-text" data-testid="import-error">{error}</p> : null}
          {rejected.length ? (
            <ul className="reject-list" data-testid="import-rejected">
              {rejected.map((item) => (
                <li key={item.name}><strong className="file-name">{item.name}</strong><span>{item.reason}</span></li>
              ))}
            </ul>
          ) : null}
        </div>

        <footer className="drawer-foot">
          <button className="ghost" onClick={close} disabled={busy}>取消</button>
          <button
            className="primary"
            data-testid="import-start"
            disabled={busy || checking || !files.length}
            onClick={start}
          >
            {busy ? "正在建立条目…" : checking ? "正在检查文件…" : splitPlans.length ? "确认导入" : `开始导入${files.length > 1 ? ` ${files.length} 份` : ""}`}
          </button>
        </footer>
      </aside>
      {pending ? (
        <ListeningAudioDialog
          key={pending.file.path}
          paperName={pending.file.name}
          cues={pending.cues}
          onDecide={decide}
          onCancel={() => {
            setQueue([]);
            setDecisions({});
          }}
        />
      ) : null}
    </div>
  );
}
