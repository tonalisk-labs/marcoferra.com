// Stripe's webhook: an unsigned or forged event is refused with 400, a redelivery books
// nothing twice, and a delayed payment books when it succeeds or frees the time when it fails.
import { apiTest, calendar, expect, mailbox } from '@tonalisk-labs/kit/testing';

import { type Caller, callerFor, confirmPaid, raw, sentTo, services } from '../support/api.ts';
import { mailOf } from '../support/mail.ts';
import { fakeStripe, holdPaid } from '../support/payment.ts';

const COMPLETED = 'checkout.session.completed';

/** The webhook's answer to an event signed with `signature`, or with none. */
const posted = async (caller: Caller, body: string, signature?: string): Promise<number> => {
  const headers: Record<string, string> =
    signature === undefined
      ? { 'content-type': 'application/json' }
      : { 'content-type': 'application/json', 'stripe-signature': signature };
  const answer = await raw(caller, '/stripe/webhook', { method: 'POST', headers, body });
  return answer.status;
};

apiTest(
  'an unsigned or forged event is refused, and a repeated one books once',
  { tag: '@api' },
  async ({ stack }) => {
    const { google, mailpit } = services(stack);
    const stripe = fakeStripe(stack);
    const caller = callerFor(stack);
    const { held, request, session } = await holdPaid(caller, 'webhook');
    const object = { id: session, status: 'complete', payment_status: 'paid' };
    const forged = JSON.stringify({ id: 'evt_forged', type: COMPLETED, data: { object } });
    const now = String(Math.floor(Date.now() / 1000));
    const answers = await Promise.all([
      posted(caller, forged),
      posted(caller, forged, 't=1,v1=00'),
      posted(caller, forged, `t=${now},v1=ab`),
    ]);
    expect(answers, 'unsigned, stale and forged').toEqual([400, 400, 400]);
    expect(await calendar(google).event(held.id)).toMatchObject({
      extendedProperties: { private: { mfState: 'held' } },
    });

    const arriving = mailbox(mailpit).waitFor(request.email);
    await stripe.pay(session);
    expect(await stripe.delivered(COMPLETED, session)).toEqual([200]);
    await arriving;
    expect(await stripe.redeliver(COMPLETED, session), 'a retry is acknowledged').toBe(200);
    expect(await sentTo(mailpit, request.email), 'and books nothing twice').toHaveLength(1);
  },
);

apiTest(
  'a delayed payment is pending, then books when it succeeds',
  { tag: '@api' },
  async ({ stack }) => {
    const { mailpit } = services(stack);
    const stripe = fakeStripe(stack);
    const caller = callerFor(stack);
    const { held, request, session } = await holdPaid(caller, 'delayed');
    await stripe.payLater(session);
    expect(await stripe.delivered(COMPLETED, session)).toEqual([200]);
    const pending = await confirmPaid(caller, session);
    expect(pending.status, 'the success page says it is pending').toBe(202);
    expect(await sentTo(mailpit, request.email)).toEqual([]);

    const arriving = mailbox(mailpit).waitFor(request.email);
    await stripe.settle(session, 'succeed');
    const succeeded = await stripe.delivered('checkout.session.async_payment_succeeded', session);
    expect(succeeded).toEqual([200]);
    const { text } = await mailOf(mailpit, await arriving);
    expect(text).toContain('by MB WAY');
    const confirmed = await confirmPaid(caller, session);
    expect(confirmed).toMatchObject({ status: 200, data: { id: held.id } });
  },
);

apiTest(
  'a delayed payment that fails frees the time and tells the visitor',
  { tag: '@api' },
  async ({ stack }) => {
    const { google, mailpit } = services(stack);
    const stripe = fakeStripe(stack);
    const { held, request, session } = await holdPaid(callerFor(stack), 'delayed-fail');
    await stripe.payLater(session);
    const arriving = mailbox(mailpit).waitFor(request.email);
    await stripe.settle(session, 'fail');
    expect(await stripe.delivered('checkout.session.async_payment_failed', session)).toEqual([200]);
    const { Subject } = await arriving;
    expect(Subject).toBe('Payment not completed');
    expect(await calendar(google).event(held.id)).toMatchObject({ status: 'cancelled' });
  },
);
