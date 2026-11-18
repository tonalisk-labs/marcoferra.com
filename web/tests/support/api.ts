// The booking API from Node, through the site's generated client. Each caller has its own edge
// cookie, so its own client address and request budget.
import { randomUUID } from 'node:crypto';
import { setTimeout as delay } from 'node:timers/promises';
import { setOrigin } from './node-app.ts';

import {
  calendar,
  type Coordinates,
  edgeIdentity,
  mailbox,
  type Summary,
} from '@tonalisk-labs/kit/testing';

import {
  type Availability,
  type BookingRequest,
  cancelBooking,
  type cancelBookingResponse,
  confirmBooking,
  type confirmBookingResponse,
  createBooking,
  type createBookingResponse,
  getAvailability,
  type getAvailabilityResponse,
  getBooking,
  type getBookingResponse,
} from '../../src/api/endpoints.ts';
import type { EventType } from '../../src/booking/event-types.ts';
import { MIN_FILL_SECONDS } from '../app.ts';
import { MEETING } from './meeting.ts';

export interface Caller {
  readonly stack: Coordinates;
  readonly cookie: string;
}

export const callerFor = (stack: Coordinates): Caller => {
  setOrigin(stack.baseURL);
  return { stack, cookie: edgeIdentity('wired').cookie };
};

export const merge = <Base extends object, Extra extends object>(
  base: Base,
  extra: Extra,
): Base & Extra => Object.assign(structuredClone(base), extra);

/** Request options carrying the caller's edge identity, and any other header. */
const as = (caller: Caller, headers: Readonly<Record<string, string>> = {}): RequestInit => ({
  headers: merge({ cookie: caller.cookie }, headers),
});

interface Plain {
  readonly method: string;
  readonly headers: Readonly<Record<string, string>>;
  readonly body: string;
}

/** A plain request through the edge, for what the client cannot send: a malformed one. */
export const raw = async (
  caller: Caller,
  path: string,
  { method, headers, body }: Plain,
): Promise<Response> =>
  fetch(`${caller.stack.baseURL}/api${path}`, {
    method,
    headers: merge(headers, { cookie: caller.cookie }),
    body,
  });

/** The availability answer for a meeting type, whatever its status. */
export const availabilityReply = async (
  caller: Caller,
  type = MEETING.slug,
): Promise<getAvailabilityResponse> => getAvailability({ type }, as(caller));

export const availability = async (caller: Caller, type = MEETING.slug): Promise<Availability> => {
  const reply = await availabilityReply(caller, type);
  if (reply.status !== 200) {
    throw new Error(`availability answered ${String(reply.status)}`);
  }
  return reply.data;
};

export const freeStarts = async (stack: Coordinates, type = MEETING.slug): Promise<string[]> => {
  const { slots } = await availability(callerFor(stack), type);
  return slots;
};

/** The form's fill time, waited out: the server refuses a form sent sooner, and nothing announces
 *  the moment it passes, so this is the one wait that is a timer. */
export const pastFillTime = async (): Promise<void> => {
  await delay(MIN_FILL_SECONDS * 1000 + 100);
};

export const visitor = (label: string): { readonly name: string; readonly email: string } => ({
  name: `Journey ${label}`,
  email: `${label}-${randomUUID()}@journeys.example`,
});

/** A start of `kind` no other test will use, clear of every other claim: the earliest starts reach
 *  the lead time during a long run, so are skipped, and the rest are tried from a random place. */
export const claimStart = async (
  stack: Coordinates,
  kind: EventType = MEETING,
): Promise<string> => {
  if (stack.google === undefined) {
    throw new Error('the stack runs no fake Google Calendar');
  }
  const starts = await freeStarts(stack, kind.slug);
  const usable = starts.slice(8);
  const from = Math.floor(Math.random() * usable.length);
  const span = {
    before: kind.buffer_minutes,
    after: kind.duration_minutes + kind.buffer_minutes,
  };
  return calendar(stack.google).claim([...usable.slice(from), ...usable.slice(0, from)], span);
};

/** A fresh visitor's booking of a claimed start, sendable once the fill time has passed; a
 *  paid type's carries the withdrawal waiver. */
export const readyBooking = async (
  caller: Caller,
  label: string,
  kind: EventType = MEETING,
): Promise<BookingRequest> => {
  const { formToken } = await availability(caller, kind.slug);
  const start = await claimStart(caller.stack, kind);
  await pastFillTime();
  return merge(visitor(label), {
    type: kind.slug,
    start,
    formToken,
    timeZone: 'Europe/Lisbon',
    withdrawalWaiver: kind.price_minor > 0,
  });
};

export const book = async (
  caller: Caller,
  request: BookingRequest,
  key = randomUUID(),
): Promise<createBookingResponse> => createBooking(request, as(caller, { 'idempotency-key': key }));

/** The booking a paid session made, confirmed as the success page confirms it. */
export const confirmPaid = async (
  caller: Caller,
  sessionId: string,
): Promise<confirmBookingResponse> => confirmBooking({ sessionId }, as(caller));

/** Cancels as the page does: with what the cancellation link names, which its token signs. */
export const cancel = async (caller: Caller, link: string): Promise<cancelBookingResponse> => {
  const query = new URL(link, 'https://www.marcoferra.com').searchParams;
  const refund = query.get('refund');
  const request = {
    token: query.get('token') ?? '',
    type: query.get('type'),
    refund: refund === null ? refund : Number(refund),
  };
  return cancelBooking(query.get('id') ?? '', request, as(caller));
};

/** The booking as its calendar file, asked for by its media type. */
export const ics = async (caller: Caller, id: string, token: string): Promise<getBookingResponse> =>
  getBooking(id, { token }, as(caller, { accept: 'text/calendar' }));

export const tokenOf = (link: string): string =>
  new URL(link, 'https://www.marcoferra.com').searchParams.get('token') ?? '';

export const services = (
  stack: Coordinates,
): { readonly google: string; readonly mailpit: string } => {
  if (stack.google === undefined || stack.mailpit === undefined) {
    throw new Error('the stack runs no fake Google Calendar or no mail sink');
  }
  return { google: stack.google, mailpit: stack.mailpit };
};

export const sentTo = async (api: string, address: string): Promise<Summary[]> => {
  const sent = await mailbox(api).list();
  return sent.filter(({ To }) => To.some(({ Address }) => Address === address));
};

/** An instant as iCalendar writes it in UTC: `20261007T100000Z`. */
export const basic = (instant: string | number): string =>
  new Date(instant).toISOString().replaceAll(/[-:]|\.\d+/gu, '');
