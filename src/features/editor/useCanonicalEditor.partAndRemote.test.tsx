// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, fireEvent, render, renderHook, screen, waitFor } from "@testing-library/react";
const applyEditorCommands = vi.fn();
const getWorkspaceItem = vi.fn();
vi.mock("../../api/workspaceClient", () => ({ applyEditorCommands: (...args: unknown[]) => applyEditorCommands(...args), getWorkspaceItem: (...args: unknown[]) => getWorkspaceItem(...args) }));
import { useCanonicalEditor } from "./useCanonicalEditor";
const workspace = (version = 1, category = 'P1', origin = 'human') => ({ item: { title: 'T' }, ds: { schemaVersion: 'IeltsAuthoringIRV2', exam: { title: 'T', category }, taskGroups: [], answerSlots: {}, answerKey: {} }, editVersion: version, recentEdits: version > 1 ? [{ baseVersion: 1, origin }] : [] });
function pending<T>() { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
beforeEach(() => { localStorage.clear(); applyEditorCommands.mockReset(); getWorkspaceItem.mockReset(); getWorkspaceItem.mockResolvedValue(workspace()); });
afterEach(cleanup);
describe('Part persistence and remote notifications', () => {
  it('holds an expected N+1 event until a successful save proves it is ours, without ever showing a warning', async () => {
    const save = pending<{ editVersion: number }>(); applyEditorCommands.mockReturnValue(save.promise);
    const { result } = renderHook(() => useCanonicalEditor('part'));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.applyPatch({ op: 'setExamCategory', category: 'P3' }));
    let flushed!: Promise<void>; act(() => { flushed = result.current.flush(); });
    await waitFor(() => expect(applyEditorCommands).toHaveBeenCalledOnce());
    act(() => result.current.noteRemoteVersion(2));
    expect(result.current.deferredRemoteRefresh).toBeUndefined();
    await act(async () => { save.resolve({ editVersion: 2 }); await flushed; });
    expect(result.current.deferredRemoteRefresh).toBeUndefined();
    expect(result.current.draft!.exam.category).toBe('P3');
    expect(getWorkspaceItem).toHaveBeenCalledOnce();
  });
  it('does not swallow another writer at the same expected version when our save conflicts', async () => {
    const save = pending<never>(); applyEditorCommands.mockReturnValue(save.promise);
    const { result } = renderHook(() => useCanonicalEditor('part'));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.applyPatch({ op: 'setExamCategory', category: 'P3' }));
    let flushed!: Promise<void>; act(() => { flushed = result.current.flush().catch(() => {}); });
    await waitFor(() => expect(applyEditorCommands).toHaveBeenCalledOnce());
    act(() => result.current.noteRemoteVersion(2));
    getWorkspaceItem.mockResolvedValue(workspace(2, 'P2'));
    await act(async () => { save.reject(new Error('EDIT_VERSION_CONFLICT:current=2:base=1')); await flushed; });
    expect(result.current.deferredRemoteRefresh).toEqual({ version: 2 });
    expect(result.current.saveState).toBe('conflict');
    expect(result.current.saveMessage).toContain('本地修改仍保留');
    expect(result.current.draft!.exam.category).toBe('P3');
    expect(result.current.pendingCount).toBe(1);
  });
  it('shows genuine higher/unknown remote updates while dirty and clears an absorbed deferred version after flush', async () => {
    applyEditorCommands.mockResolvedValue({ editVersion: 2 });
    const { result } = renderHook(() => useCanonicalEditor('part'));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.applyPatch({ op: 'setExamCategory', category: 'P2' }));
    act(() => result.current.noteRemoteVersion(2)); // No save request yet: genuine external event.
    expect(result.current.deferredRemoteRefresh).toEqual({ version: 2 });
    await act(async () => { await result.current.flush(); });
    expect(result.current.deferredRemoteRefresh).toBeUndefined();
    expect(getWorkspaceItem).toHaveBeenCalledOnce();
  });
  it('reloads a genuine higher event after our save and keeps unknown versions conservative', async () => {
    const save = pending<{ editVersion: number }>(); applyEditorCommands.mockReturnValue(save.promise);
    const { result } = renderHook(() => useCanonicalEditor('part'));
    await waitFor(() => expect(result.current.loading).toBe(false));
    act(() => result.current.applyPatch({ op: 'setExamCategory', category: 'P3' }));
    let flushed!: Promise<void>; act(() => { flushed = result.current.flush(); });
    await waitFor(() => expect(applyEditorCommands).toHaveBeenCalledOnce());
    act(() => result.current.noteRemoteVersion(3));
    expect(result.current.deferredRemoteRefresh).toEqual({ version: 3 });
    act(() => result.current.noteRemoteVersion(undefined));
    act(() => result.current.noteRemoteVersion(2));
    expect(result.current.deferredRemoteRefresh).toEqual({ version: undefined });
    getWorkspaceItem.mockResolvedValue(workspace(3, 'P2'));
    await act(async () => { save.resolve({ editVersion: 2 }); await flushed; });
    await waitFor(() => expect(result.current.version).toBe(3));
    expect(result.current.draft!.exam.category).toBe('P2');
    expect(result.current.deferredRemoteRefresh).toBeUndefined();
  });
  it.each(['CLOUD_REVIEW_IN_PROGRESS:part', 'validation_failed', 'EDIT_VERSION_CONFLICT:current=2:base=1'])('keeps the Part selection and explains unsuccessful save: %s', async error => {
    applyEditorCommands.mockRejectedValue(new Error(error));
    function PartControl() {
      const editor = useCanonicalEditor('part');
      return <><select aria-label="Part" value={editor.draft?.exam.category ?? ''} onChange={event => editor.applyPatch({ op: 'setExamCategory', category: event.target.value as 'P3' })}><option>P1</option><option>P3</option></select>{editor.saveMessage ? <p role="alert">{editor.saveMessage}</p> : null}<button onClick={() => void editor.flush().catch(() => {})}>Save</button></>;
    }
    render(<PartControl />);
    await waitFor(() => expect((screen.getByLabelText('Part') as HTMLSelectElement).value).toBe('P1'));
    fireEvent.change(screen.getByLabelText('Part'), { target: { value: 'P3' } });
    fireEvent.click(screen.getByText('Save'));
    await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('保留'));
    expect((screen.getByLabelText('Part') as HTMLSelectElement).value).toBe('P3');
    expect(localStorage.getItem('ielts-author-studio.workspace-recovery.v1:part')).toContain('P3');
  });
});
