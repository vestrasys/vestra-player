/** Headless players refuse the display; that never fails the action itself. */
export const quiet = (shown: Promise<unknown>) => void shown.catch(() => {});
