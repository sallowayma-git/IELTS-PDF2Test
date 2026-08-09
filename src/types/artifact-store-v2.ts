export interface CurrentRevisionV2 {
  schemaVersion: "JobCurrentRevisionV1";
  layoutVersion: "JobArtifactLayoutV1";
  jobId: string;
  revision: number;
  updatedAt: string;
}

export interface RevisionRecordV2 {
  schemaVersion: "AuthoringRevisionRecordV1";
  layoutVersion: "JobArtifactLayoutV1";
  jobId: string;
  revision: number;
  parentRevision: number;
  source: "auto_extract" | "user" | "migration";
  createdAt: string;
  artifactPath: string;
  artifactSha256: string;
  patchPath: string | null;
  patchSha256: string | null;
}

export interface JobArtifactStatusV2 {
  schemaVersion: "JobArtifactStatusV1";
  layoutVersion: "JobArtifactLayoutV1";
  jobId: string;
  current: CurrentRevisionV2;
  revisions: RevisionRecordV2[];
  paths: {
    sources: string;
    extraction: string;
    authoring: string;
    assets: string;
    preview: string;
    exportHistory: string;
    legacy: string;
  };
  v1FilesRemainReadable: true;
}
