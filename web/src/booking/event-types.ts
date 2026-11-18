// The meeting types of marcoferra.config.json, checked as the site builds by the rules the server
// holds the file to (server/src/config.rs).
import site from '../../../marcoferra.config.json' with { type: 'json' };

/** Stripe's names for the ways a paid type is paid, as the server reads them. */
const PAYMENT_METHODS = ['card', 'mb_way', 'paypal'] as const;
type PaymentMethod = (typeof PAYMENT_METHODS)[number];

/** An entry as the file holds it, its payment methods still plain strings. */
type Entry = Omit<(typeof site.event_types)[number], 'payment_methods'> & {
  readonly payment_methods: readonly string[];
};

/** A meeting type, its payment methods each one the server reads. */
export type EventType = Omit<Entry, 'payment_methods'> & {
  readonly payment_methods: readonly PaymentMethod[];
};

const SLUG = /^[a-z0-9-]{1,40}$/u;
/** Slugs the site's own pages under /book/ take. */
const RESERVED = new Set(['cancel']);
const MIN_PRICE_MINOR = 50;
const METHODS: readonly string[] = PAYMENT_METHODS;

const isMethod = (method: string): method is PaymentMethod => METHODS.includes(method);

const fault = (entry: Entry, index: number, all: readonly Entry[]): string | undefined => {
  const { slug, duration_minutes: minutes, price_minor: price, payment_methods: methods } = entry;
  const rules: readonly (readonly [boolean, string])[] = [
    [
      SLUG.test(slug) && !RESERVED.has(slug),
      'the slug needs 1 to 40 of a-z, 0-9 or -, and not cancel',
    ],
    [all.findIndex((other) => other.slug === slug) === index, 'the slug is used twice'],
    [
      [entry.title, entry.description, entry.summary].every((text) => text.trim() !== ''),
      'the title, description and summary need text',
    ],
    [minutes > 0 && minutes % 5 === 0, 'duration_minutes must be a positive multiple of 5'],
    [
      price === 0 || (price >= MIN_PRICE_MINOR && methods.length > 0),
      'price_minor is 0, or at least 50 with payment_methods',
    ],
    [methods.every((method) => isMethod(method)), 'an unknown payment method'],
    [!methods.includes('mb_way') || entry.currency === 'eur', 'MB WAY takes eur'],
  ];
  const broken = rules.find(([kept]) => !kept);
  return broken === undefined ? undefined : broken[1];
};

/** The meeting types, in the file's order; a broken entry stops the build. */
export const eventTypes = (entries: readonly Entry[] = site.event_types): EventType[] => {
  if (entries.length === 0) {
    throw new Error('marcoferra.config.json: event_types lists no meeting type');
  }
  return entries.map((entry, index) => {
    const broken = fault(entry, index, entries);
    if (broken !== undefined) {
      throw new Error(`marcoferra.config.json: event_types ${entry.slug}: ${broken}`);
    }
    const methods = entry.payment_methods.filter((method) => isMethod(method));
    return Object.assign(structuredClone(entry), { payment_methods: methods });
  });
};

export const durationOf = ({ duration_minutes: minutes }: EventType): string =>
  `${String(minutes)} minutes`;

/** An amount as English writes it in its currency, without the cents of a whole one: €40. */
export const money = (minor: number, currency: string): string => {
  const format = new Intl.NumberFormat('en', {
    style: 'currency',
    currency: currency.toUpperCase(),
    trailingZeroDisplay: 'stripIfInteger',
  });
  return format.format(minor / 10 ** (format.resolvedOptions().maximumFractionDigits ?? 2));
};

export const priceOf = ({ price_minor: price, currency }: EventType): string =>
  price === 0 ? 'Free' : money(price, currency);

/** How a person names each way to pay; a card brings Apple Pay and Google Pay with it. */
const METHOD_NAMES: Readonly<Record<PaymentMethod, readonly string[]>> = {
  card: ['card', 'Apple Pay', 'Google Pay'],
  mb_way: ['MB WAY'],
  paypal: ['PayPal'],
};

/** A type's ways to pay, as a sentence lists them: "card, Apple Pay, …, or PayPal". */
export const methodsOf = ({ payment_methods: methods }: EventType): string =>
  new Intl.ListFormat('en', { type: 'disjunction' }).format(
    methods.flatMap((method) => METHOD_NAMES[method]),
  );

export const paidTypes = (): EventType[] => eventTypes().filter((kind) => kind.price_minor > 0);
