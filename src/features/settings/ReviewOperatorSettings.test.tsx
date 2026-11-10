// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { command } from "../../api/tauriCommands";
import { ReviewOperatorSettings } from "./ReviewOperatorSettings";

vi.mock("../../api/tauriCommands", () => ({ command: vi.fn() }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it("loads the persisted operator and saves the edited name through the real command contract", async () => {
  vi.mocked(command).mockResolvedValueOnce({ operatorName: "张老师" }).mockResolvedValueOnce({ operatorName: "李老师" });
  render(<ReviewOperatorSettings />);
  await screen.findByDisplayValue("张老师");
  fireEvent.change(screen.getByLabelText("操作人姓名"), { target: { value: " 李老师 " } });
  fireEvent.click(screen.getByRole("button", { name: "保存操作人" }));
  await screen.findByRole("status");
  expect(command).toHaveBeenLastCalledWith("save_review_settings", { input: { operatorName: "李老师" } });
  expect(screen.getByDisplayValue("李老师")).toBeTruthy();
});

it("does not report a failed save as successful", async () => {
  vi.mocked(command).mockResolvedValueOnce({ operatorName: "张老师" }).mockRejectedValueOnce(new Error("write_failed"));
  render(<ReviewOperatorSettings />);
  await screen.findByDisplayValue("张老师");
  await waitFor(() => expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(screen.getByRole("button"));
  await screen.findByRole("alert");
  expect(screen.queryByRole("status")).toBeNull();
});
