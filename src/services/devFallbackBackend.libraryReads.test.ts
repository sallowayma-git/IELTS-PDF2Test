// @vitest-environment jsdom
import { beforeEach, expect, it } from "vitest";
import { devFallbackInvoke } from "./devFallbackBackend";
import type { LibraryRowData } from "../api/workspaceClient";
import type { ImportJob, WritingJob } from "../types";

beforeEach(() => localStorage.clear());

it("single-row fallback preserves reading metadata and trash state", async () => {
  const job = await devFallbackInvoke<ImportJob>("create_import_job", { input: { title: "Reading", category: "practice" } });
  const row = await devFallbackInvoke<LibraryRowData>("get_library_row", { itemId: job.jobId });
  expect(row.job?.jobId).toBe(job.jobId);
  expect(row.summary?.title).toBe("Reading");
  expect(row.inTrash).toBe(false);
  await devFallbackInvoke("delete_library_exam", { id: job.jobId });
  expect((await devFallbackInvoke<LibraryRowData>("get_library_row", { itemId: job.jobId })).inTrash).toBe(true);
  expect(await devFallbackInvoke("get_library_item_processing", { itemId: job.jobId })).toBeNull();
});

it("single-row fallback preserves writing modality without a reading job", async () => {
  const job = await devFallbackInvoke<WritingJob>("create_writing_job", { input: { title: "Writing", taskType: "task2" } });
  const row = await devFallbackInvoke<LibraryRowData>("get_library_row", { itemId: job.jobId });
  expect(row.job).toBeNull();
  expect(row.summary).toMatchObject({ title: "Writing", subject: "writing", taskType: "task2" });
  expect(row.inTrash).toBe(false);
  expect(await devFallbackInvoke("get_library_row", { itemId: "missing" })).toBeNull();
});
