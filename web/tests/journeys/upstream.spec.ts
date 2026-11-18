// Upstream failure: a failing calendar or a refusing relay leaves nothing booked and the time
// free, and a retry with the same key books. Each owns the stack, as it steers shared services.
import { apiTest, calendar, expect, mailbox } from '@tonalisk-labs/kit/testing';

import { book, callerFor, freeStarts, readyBooking, services } from '../support/api.ts';

apiTest(
  'a calendar that fails leaves nothing booked, and a retry books',
  { tag: ['@api', '@exclusive'] },
  async ({ stack }) => {
    const { google } = services(stack);
    const caller = callerFor(stack);
    const request = await readyBooking(caller, 'calendar-down');
    const key = crypto.randomUUID();
    await calendar(google).fail('freeBusy', 500);
    const unread = await book(caller, request, key);
    expect(unread.status).toBe(503);
    await calendar(google).fail('insert', 503);
    const unwritten = await book(caller, request, key);
    expect(unwritten.status).toBe(503);
    const retried = await book(caller, request, key);
    expect(retried.status).toBe(201);
  },
);

apiTest(
  'an invitation the relay refuses withdraws the event, and a retry books the time',
  { tag: ['@api', '@exclusive'] },
  async ({ stack }) => {
    const inbox = mailbox(services(stack).mailpit);
    const caller = callerFor(stack);
    const request = await readyBooking(caller, 'relay-down');
    const key = crypto.randomUUID();
    await inbox.refuseRecipients(451);
    try {
      const refused = await book(caller, request, key);
      expect(refused.status).toBe(503);
    } finally {
      await inbox.acceptRecipients();
    }
    expect(await freeStarts(stack), 'the withdrawn time is free again').toContain(request.start);
    const arriving = inbox.waitFor(request.email);
    const retried = await book(caller, request, key);
    expect(retried.status).toBe(201);
    const arrived = await arriving;
    expect(arrived.To.map(({ Address }) => Address)).toEqual([request.email]);
  },
);
