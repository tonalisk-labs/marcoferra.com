import {
  type Availability,
  type Booking,
  confirmBooking,
  createBooking,
  getAvailability,
  type getAvailabilityResponse,
  type Held,
  type Problem,
  releaseBooking,
} from '../api/endpoints.ts';
import { type Fault, replyOf, TransportError } from '../api/transport.ts';
import { messageOf } from './brief.ts';
import { takeEarlyRead } from './early-read.ts';

export interface Refusal {
  readonly message: string;
  /** Messages by field name, from the problem's JSON Pointers. */
  readonly fields: ReadonlyMap<string, string>;
  /** Whether this was a settled answer, so the next attempt is a new one (a new key). */
  readonly settled: boolean;
  /** Whether the free times have changed since they were read. */
  readonly stale: boolean;
}

const SAID: Record<Fault | 'fault', string> = {
  offline:
    "Your connection seems to be down, so I couldn't reach the booking service. Check it and try again.",
  unavailable: "The booking service isn't answering right now. Give it a minute and try again.",
  fault:
    'Something went wrong on my side. Try again, and if it keeps happening, just send me an email.',
};

export const failureOf = (error: unknown): string =>
  SAID[error instanceof TransportError ? error.fault : 'fault'];

/** A failed request as a refusal that the same attempt may retry. */
export const unanswered = (error: unknown): Refusal => ({
  message: failureOf(error),
  fields: new Map(),
  settled: false,
  stale: false,
});

const refusalOf = (problem: Problem, settled: boolean): Refusal => ({
  message: problem.detail ?? problem.title,
  fields: new Map(
    (problem.errors ?? []).map(({ pointer, detail }) => [pointer.replace(/^\//u, ''), detail]),
  ),
  settled,
  stale: problem.type === 'tag:marcoferra.com,2026:slot-taken',
});

const freeTimesOf = (reply: getAvailabilityResponse): Availability | Refusal =>
  reply.status === 200 ? reply.data : refusalOf(reply.data, false);

export const loadAvailability = async (type: string): Promise<Availability | Refusal> =>
  freeTimesOf(await getAvailability({ type }));

/** The first read: the one the page started while it loaded, else a new one. */
export const firstAvailability = async (type: string): Promise<Availability | Refusal> => {
  const early = takeEarlyRead();
  if (early === undefined) {
    return loadAvailability(type);
  }
  const response = await early;
  if (response === undefined) {
    throw new TransportError('offline');
  }
  return freeTimesOf(await replyOf<getAvailabilityResponse>(response));
};

const text = (form: FormData, name: string): string => {
  const value = form.get(name);
  return typeof value === 'string' ? value : '';
};

export interface Attempt {
  /** The meeting type's slug. */
  readonly type: string;
  readonly zone: string;
  readonly formToken: string;
  /** The Idempotency-Key: the same for every retry of one attempt. */
  readonly key: string;
}

export type Outcome =
  | { readonly booked: Booking; readonly email: string }
  | { readonly held: Held }
  | Refusal;

const KEEPABLE = 'abcdefghijklmnopqrstuvwxyz0123456789.-';

/** The server keeps 1–32 of a–z, 0–9, "." and "-", the same rule it validates. */
const keepable = (source: string): boolean => {
  if (source.length === 0 || source.length > 32) {
    return false;
  }
  for (const char of source) {
    if (!KEEPABLE.includes(char)) {
      return false;
    }
  }
  return true;
};

/** The link's `utm_source`, when it's one the server keeps. Any other is left out, so a stray tag
 *  never refuses a booking. */
const sourceOf = (search: string): string | undefined => {
  const source = (new URLSearchParams(search).get('utm_source') ?? '').trim().toLowerCase();
  return keepable(source) ? source : undefined;
};

const source = (search: string): { source?: string } => {
  const named = sourceOf(search);
  return named === undefined ? {} : { source: named };
};

/** The brief, where the type asks for one, travels in the message. */
export const book = async (
  form: FormData,
  { type, zone, formToken, key }: Attempt,
): Promise<Outcome> => {
  const email = text(form, 'email').trim();
  const fields = {
    type,
    start: text(form, 'start'),
    name: text(form, 'name'),
    email,
    message: messageOf(form, type),
    timeZone: zone,
    website: text(form, 'website'),
    formToken,
    withdrawalWaiver: form.get('withdrawalWaiver') === 'on',
  };
  const request = Object.assign(structuredClone(fields), source(globalThis.location.search));
  const reply = await createBooking(request, { headers: { 'Idempotency-Key': key } });
  if (reply.status === 201) {
    return { booked: reply.data, email };
  }
  if (reply.status === 202) {
    return { held: reply.data };
  }
  return refusalOf(reply.data, reply.status === 400 || reply.status === 409);
};

/** Gives up a held time from Checkout's way back; true once it is free again. */
export const release = async (id: string, token: string): Promise<boolean> => {
  const reply = await releaseBooking(id, { token });
  return reply.status === 204;
};

export type Confirmed =
  | { readonly booked: Booking }
  | { readonly pending: string }
  | { readonly refusal: string };

/** The booking a returning payment made, as the server confirms it. */
export const confirm = async (sessionId: string): Promise<Confirmed> => {
  const reply = await confirmBooking({ sessionId });
  if (reply.status === 200) {
    return { booked: reply.data };
  }
  if (reply.status === 202) {
    return { pending: reply.data.detail };
  }
  return { refusal: reply.data.detail ?? reply.data.title };
};
