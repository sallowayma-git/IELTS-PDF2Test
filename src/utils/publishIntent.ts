export type PublishIntent =
  | { mode: "nas-library"; jobId?: string }
  | { mode: "writing-library" };

const PUBLISH_INTENT_KEY = "ielts-author-studio.publish-intent";

export function setPublishIntent(intent: PublishIntent): void {
  window.sessionStorage.setItem(PUBLISH_INTENT_KEY, JSON.stringify(intent));
}

export function takePublishIntent(): PublishIntent | undefined {
  const raw = window.sessionStorage.getItem(PUBLISH_INTENT_KEY);
  if (!raw) return undefined;
  window.sessionStorage.removeItem(PUBLISH_INTENT_KEY);
  try {
    return JSON.parse(raw) as PublishIntent;
  } catch {
    return undefined;
  }
}
