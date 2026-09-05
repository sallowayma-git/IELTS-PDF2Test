import { useState } from "react";
import { choosePdfFolderSources, chooseSourceFiles, type PickedPath } from "../../api/desktopDialogs";
import type { ImportRejection } from "./useImportFiles";

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
  onImport: (files: PickedPath[]) => void;
}) {
  const [files, setFiles] = useState<PickedPath[]>([]);

  if (!open) return null;

  const addFiles = (picked: PickedPath[]) => {
    if (!picked.length) return;
    setFiles((current) => {
      const byPath = new Map(current.map((file) => [file.path, file]));
      for (const file of picked) byPath.set(file.path, file);
      return [...byPath.values()];
    });
  };

  const remove = (path: string) => setFiles((current) => current.filter((file) => file.path !== path));

  const close = () => {
    setFiles([]);
    onClose();
  };

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
            disabled={busy || !files.length}
            onClick={() => onImport(files)}
          >
            {busy ? "正在建立条目…" : `开始导入${files.length > 1 ? ` ${files.length} 份` : ""}`}
          </button>
        </footer>
      </aside>
    </div>
  );
}
