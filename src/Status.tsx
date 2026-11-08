// A status message shown after loading/saving - `isError` picks the CSS class so a failed save
// (e.g. the write-safety gate refusing it because the server is still running) can never look
// identical to a successful one. A real, reported problem this exists to fix: a save silently
// rejected this way looked exactly like any other status line and was missed entirely, leaving
// the owner believing a field change had been saved when the database was completely untouched.
export type StatusState = { text: string; isError: boolean };
export const emptyStatus: StatusState = { text: "", isError: false };
export const okStatus = (text: string): StatusState => ({ text, isError: false });
export const errorStatus = (text: string): StatusState => ({ text, isError: true });

export function StatusMessage({ status }: { status: StatusState }) {
  if (!status.text) return null;
  return <pre className={`status${status.isError ? " status-error" : ""}`}>{status.text}</pre>;
}
