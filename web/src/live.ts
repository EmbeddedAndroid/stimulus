// Live-view helpers: decide when the timeline should poll for fresh data and
// which capture the timeline should show. Pure functions so they can be unit
// tested without a DOM or a running daemon.

/// Acquisition phases during which a capture is still filling. While in one of
/// these (or while a recurring run is active) the UI polls fast so the waveform
/// and buffer-fill update smoothly; otherwise it still polls at a calm baseline
/// so freshly finished captures appear on their own.
export const ACTIVE_ACQ_STATES: ReadonlySet<string> = new Set([
  "prefill",
  "armed",
  "postfill",
]);

export interface LiveAcquisition {
  state: string;
  recurring: boolean;
}

/// True when a capture is in progress (or a recurring acquisition is running).
export function isLiveState(acq: LiveAcquisition | null): boolean {
  if (acq === null) return false;
  return acq.recurring || ACTIVE_ACQ_STATES.has(acq.state);
}

/// Poll cadence in ms: brisk while a capture fills, calm while idle. The idle
/// cadence is what makes the app feel live without a manual refresh.
export function pollIntervalMs(live: boolean): number {
  return live ? 300 : 1500;
}

/// The id of the most recent capture (highest id), independent of list order.
export function newestId(captures: readonly { id: number }[]): number | null {
  if (captures.length === 0) return null;
  return captures.reduce((max, capture) => (capture.id > max ? capture.id : max), captures[0]!.id);
}

/// Captures newest-first, so the latest always sits at the top of the history.
export function sortedByNewest<T extends { id: number }>(captures: readonly T[]): T[] {
  return [...captures].sort((a, b) => b.id - a.id);
}

/// Which capture id the timeline should display after a refresh. When the view
/// is "following" (the default) it snaps to the newest capture so a running or
/// recurring acquisition always shows its latest result; once the user pins an
/// older capture from history, following is off and their choice is preserved.
export function nextSelectedId(
  captures: readonly { id: number }[],
  current: number | null,
  following: boolean,
): number | null {
  const newest = newestId(captures);
  return following ? newest ?? current : current ?? newest;
}
