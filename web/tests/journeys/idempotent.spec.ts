// An idempotent retry: the same Idempotency-Key answers the booking it made and sends no
// second invitation; the key sent again with another time is refused.
import { apiTest, expect, mailbox } from '@tonalisk-labs/kit/testing';

import { book, callerFor, claimStart, merge, readyBooking, services } from '../support/api.ts';

apiTest(
  'a retried booking answers the same booking and sends one invitation',
  { tag: '@api' },
  async ({ stack }) => {
    const { mailpit } = services(stack);
    const caller = callerFor(stack);
    const request = await readyBooking(caller, 'retry');
    const key = crypto.randomUUID();
    const first = await book(caller, request, key);
    const again = await book(caller, request, key);
    expect(first.status).toBe(201);
    expect(again).toEqual(expect.objectContaining({ status: 201, data: first.data }));
    const sent = await mailbox(mailpit).list();
    const theirs = sent.filter(({ To }) => To.some(({ Address }) => Address === request.email));
    expect(theirs).toHaveLength(1);

    const other = await claimStart(stack);
    const reused = await book(caller, merge(request, { start: other }), key);
    expect(reused).toMatchObject({
      status: 409,
      data: { type: 'tag:marcoferra.com,2026:idempotency-key-reused' },
    });
  },
);
