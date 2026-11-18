// The booking page reads the free times while the island still loads, and keeps the reply for the
// island's first read: the API serves one request at a time. book/[slug].astro hashes these bytes.

declare global {
  interface Window {
    earlyFreeTimes?: Promise<Response | undefined> | undefined;
  }
}

/** Reads `url` at once, unless the address names a hold to give up: that comes first. */
export const earlyRead = (url: string): string => `(() => {
  const query = new URLSearchParams(location.search);
  if (!query.has('release') || !query.has('token')) window.earlyFreeTimes = fetch(${JSON.stringify(url)}).catch(() => undefined);
})();`;

/** The page's read, taken once; none when the page started none. */
export const takeEarlyRead = (): Promise<Response | undefined> | undefined => {
  const early = globalThis.window.earlyFreeTimes;
  globalThis.window.earlyFreeTimes = undefined;
  return early;
};
