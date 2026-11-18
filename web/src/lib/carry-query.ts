/** Passes the query the visitor came with, such as utm_source, on to the links `selector` finds,
 *  so the meeting type they choose keeps it. */
export const carryQuery = (selector: string): void => {
  const { search } = globalThis.location;
  for (const link of globalThis.document.querySelectorAll<HTMLAnchorElement>(selector)) {
    link.search = search;
  }
};
