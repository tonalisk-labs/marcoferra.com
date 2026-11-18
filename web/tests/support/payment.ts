// The fake Stripe's controls (kit's stripe-payments.ts) each answer an event a test waits on,
// never a timer.
import type { Page } from '@playwright/test';
import { type Coordinates, expect } from '@tonalisk-labs/kit/testing';

import type { Booking, BookingRequest, Held } from '../../src/api/endpoints.ts';
import { type EventType, money, paidTypes } from '../../src/booking/event-types.ts';
import { book, type Caller, confirmPaid, pastFillTime, readyBooking } from './api.ts';
import { pay, waive } from './ui.ts';
import { fillVisit, type Visit } from './visit.ts';

const [first] = paidTypes();
if (first === undefined) {
  throw new Error('marcoferra.config.json lists no paid meeting type');
}

/** The paid type the journeys book: the first the configuration lists, and its page. */
export const PAID: EventType = first;
export const PAID_PATH = `/book/${PAID.slug}/`;
export const PRICE = money(PAID.price_minor, PAID.currency);

export const sessionOf = (checkoutUrl: string): string =>
  new URL(checkoutUrl).pathname.split('/').at(-1) ?? '';

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null;

const json = async (response: Response): Promise<Record<string, unknown>> => {
  const body: unknown = await response.json();
  return isObject(body) ? body : {};
};

interface FakeStripe {
  /** Pays at Checkout as Pay does; answers where the buyer is sent back. */
  readonly pay: (session: string) => Promise<string>;
  /** Completes with a method that settles later. */
  readonly payLater: (session: string) => Promise<void>;
  readonly settle: (session: string, outcome: 'succeed' | 'fail') => Promise<void>;
  /** Expires the session; `silent` loses the webhook Stripe would send. */
  readonly expire: (session: string, silent?: boolean) => Promise<void>;
  /** The host refunds the session's payment from the Dashboard. */
  readonly refund: (session: string) => Promise<void>;
  /** Waits until Stripe delivered `type` for the session; answers each delivery's status. */
  readonly delivered: (type: string, session: string) => Promise<number[]>;
  /** The session as Stripe holds it, with its charge. */
  readonly session: (session: string) => Promise<Record<string, unknown>>;
  /** Sends the last `type` event of the session again; answers the webhook's status. */
  readonly redeliver: (type: string, session: string) => Promise<number>;
}

/** A control that answers `status` when it did what it was told. */
const control =
  (url: string) =>
  async (path: string, status = 200): Promise<Response> => {
    const answer = await fetch(`${url}${path}`, { method: 'POST', redirect: 'manual' });
    expect(answer.status, path).toBe(status);
    return answer;
  };

/** The fake Stripe a default run starts; a sandbox run has none. */
export const fakeStripe = (stack: Coordinates): FakeStripe => {
  if (stack.stripe === undefined) {
    throw new Error('the stack runs no fake Stripe');
  }
  const url = stack.stripe;
  const post = control(url);
  return {
    pay: async (session) => {
      const paid = await post(`/checkout/${session}`, 303);
      return paid.headers.get('location') ?? '';
    },
    payLater: async (session) => {
      await post(`/fake/pay-async/${session}`, 303);
    },
    settle: async (session, outcome) => {
      await post(`/fake/async/${session}/${outcome}`);
    },
    expire: async (session, silent = false) => {
      await post(`/fake/expire/${session}${silent ? '?silent=1' : ''}`);
    },
    refund: async (session) => {
      await post(`/fake/refund/${session}`);
    },
    delivered: async (type, session) => {
      const { statuses } = await json(await fetch(`${url}/fake/delivered/${type}/${session}`));
      return Array.isArray(statuses) ? statuses.map(Number) : [];
    },
    session: async (session) =>
      json(
        await fetch(`${url}/v1/checkout/sessions/${session}?expand[]=payment_intent.latest_charge`),
      ),
    redeliver: async (type, session) => {
      const { status } = await json(await post(`/fake/redeliver/${type}/${session}`));
      return Number(status);
    },
  };
};

/** A hold made through the API for a fresh visitor, and the request that made it. */
export const holdPaid = async (
  caller: Caller,
  label: string,
): Promise<{ readonly held: Held; readonly request: BookingRequest; readonly session: string }> => {
  const request = await readyBooking(caller, label, PAID);
  const reply = await book(caller, request);
  if (reply.status !== 202) {
    throw new Error(`the hold answered ${String(reply.status)}`);
  }
  return { held: reply.data, request, session: sessionOf(reply.data.checkoutUrl) };
};

/** A paid booking made through the API: held, paid at the fake Checkout, and confirmed. */
export const bookPaid = async (
  caller: Caller,
  label: string,
): Promise<{
  readonly booked: Booking;
  readonly request: BookingRequest;
  readonly session: string;
}> => {
  const { request, session } = await holdPaid(caller, label);
  await fakeStripe(caller.stack).pay(session);
  const confirmed = await confirmPaid(caller, session);
  if (confirmed.status !== 200) {
    throw new Error(`the confirmation answered ${String(confirmed.status)}`);
  }
  return { booked: confirmed.data, request, session };
};

/** On the paid type's page, as a visitor: a claimed time, the details, the waiver, and Pay, which
 *  leaves for Checkout. */
export const holdOnPage = async (page: Page, stack: Coordinates, label: string): Promise<Visit> => {
  const visit = await fillVisit(page, stack, { label, kind: PAID });
  await waive(page);
  await pay(page, { price: PRICE, wait: pastFillTime });
  return visit;
};
