import { useEffect, useState } from "react";
import { command } from "../../api/tauriCommands";
import { toUserFacingError } from "../../utils/userFacingError";

export function ReviewOperatorSettings() {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let cancelled = false;
    command<{ operatorName: string }>("get_review_settings").then((value) => {
      if (!cancelled) setName(value.operatorName);
    }).catch((caught) => {
      if (!cancelled) setError(toUserFacingError(caught, "读取操作人失败，请在桌面应用中重试。").userMessage);
    }).finally(() => { if (!cancelled) setBusy(false); });
    return () => { cancelled = true; };
  }, []);

  async function save() {
    setBusy(true);
    setError(undefined);
    setSaved(false);
    try {
      const value = await command<{ operatorName: string }>("save_review_settings", { input: { operatorName: name.trim() } });
      setName(value.operatorName);
      setSaved(true);
    } catch (caught) {
      setError(toUserFacingError(caught, "保存操作人失败，请重试。").userMessage);
    } finally {
      setBusy(false);
    }
  }

  return <div>
    <label className="settings-field">
      操作人姓名
      <input value={name} maxLength={100} disabled={busy} onChange={(event) => { setName(event.target.value); setSaved(false); }} />
    </label>
    <small>用于记录成绩导出和批改操作，请填写实际操作人。</small>
    <button className="ghost small" disabled={busy || !name.trim()} onClick={() => void save()}>保存操作人</button>
    {saved ? <p role="status">操作人已保存</p> : null}
    {error ? <p className="error-text" role="alert">{error}</p> : null}
  </div>;
}
