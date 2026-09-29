import { describe, expect, it } from "vitest";
import type { AuthoringAuditV2 } from "./ielts-authoring-v2";
import type { RuntimeAuditV2 } from "./reading-runtime-v2";
import type { ListeningRuntimeAuditV1 } from "./listening-runtime-v1";

describe("cloud adoption audit source", () => {
  it("is accepted by authoring and both exported runtime audit contracts", () => {
    const authoringSource: AuthoringAuditV2["source"] = "cloud_candidate_adoption";
    const readingSource: RuntimeAuditV2["sourceRevisionKind"] = "cloud_candidate_adoption";
    const listeningSource: ListeningRuntimeAuditV1["sourceRevisionKind"] = "cloud_candidate_adoption";

    expect([authoringSource, readingSource, listeningSource]).toEqual([
      "cloud_candidate_adoption",
      "cloud_candidate_adoption",
      "cloud_candidate_adoption",
    ]);
  });
});
