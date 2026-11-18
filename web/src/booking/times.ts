// Times as the visitor reads them: every UTC start shown, and grouped by day, in the visitor's
// own zone; the host's grid and rules stay on the server.
import { TZDate } from '@date-fns/tz';
import { format } from 'date-fns';

export const dayOf = (instant: Date | string, zone: string): string =>
  format(new TZDate(new Date(instant).getTime(), zone), 'yyyy-MM-dd');

export const timeOf = (instant: string, zone: string): string =>
  format(new TZDate(Date.parse(instant), zone), 'HH:mm');

export const longDayOf = (instant: string, zone: string): string =>
  format(new TZDate(Date.parse(instant), zone), 'EEEE d MMMM yyyy');

/** Items by a key, in their order within each group. */
const grouped = <Item>(
  items: readonly Item[],
  key: (item: Item) => string,
): Map<string, Item[]> => {
  const groups = new Map<string, Item[]>();
  for (const item of items) {
    const name = key(item);
    const group = groups.get(name);
    if (group === undefined) {
      groups.set(name, [item]);
    } else {
      group.push(item);
    }
  }
  return groups;
};

/** Starts by the visitor's day, earliest first within each. */
export const byDay = (slots: readonly string[], zone: string): Map<string, string[]> =>
  grouped(slots, (slot) => dayOf(slot, zone));

export const detectedZone = (): string => new Intl.DateTimeFormat().resolvedOptions().timeZone;

const regionOf = (zone: string): string =>
  zone.includes('/') ? zone.slice(0, zone.indexOf('/')) : 'Other';

const regions = (zones: readonly string[]): (readonly [string, readonly string[]])[] => [
  ...grouped(zones.toSorted(), regionOf),
];

/** The browser's zone list is static, so it is sorted and grouped once, on first use. */
const known: {
  zones?: ReadonlySet<string>;
  regions?: readonly (readonly [string, readonly string[]])[];
} = {};

/** Every zone the browser knows, grouped by region for a native `<select>`. */
export const zonesByRegion = (
  current: string,
): readonly (readonly [string, readonly string[]])[] => {
  known.zones ??= new Set([...Intl.supportedValuesOf('timeZone'), 'UTC']);
  known.regions ??= regions([...known.zones]);
  return known.zones.has(current) ? known.regions : regions([...known.zones, current]);
};
