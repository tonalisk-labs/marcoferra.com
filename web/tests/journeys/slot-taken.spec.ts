// A time taken meanwhile: the next visitor is refused as slot-taken, a race makes one booking,
// and the page says so and offers what is still free.
import { apiTest, calendar, expect, test } from '@tonalisk-labs/kit/testing';

import { timeOf } from '../../src/booking/times.ts';
import {
  book,
  callerFor,
  merge,
  pastFillTime,
  readyBooking,
  services,
  visitor,
} from '../support/api.ts';
import { MEETING } from '../support/meeting.ts';
import { send } from '../support/ui.ts';
import { fillVisit } from '../support/visit.ts';

const TAKEN = 'tag:marcoferra.com,2026:slot-taken';
const MEETING_MS = MEETING.duration_minutes * 60_000;

apiTest(
  'a booked time is refused to the next visitor, and a race makes one booking',
  { tag: '@api' },
  async ({ stack }) => {
    const first = callerFor(stack);
    const request = await readyBooking(first, 'taken-first');
    const booked = await book(first, request);
    expect(booked.status).toBe(201);
    const rival = await book(callerFor(stack), merge(request, visitor('taken-second')));
    expect(rival).toMatchObject({ status: 409, data: { type: TAKEN } });

    const race = await readyBooking(first, 'race');
    const entries = ['race-a', 'race-b'].map((label) => merge(race, visitor(label)));
    const replies = await Promise.all(entries.map(async (entry) => book(callerFor(stack), entry)));
    const statuses = replies.map(({ status }) => status);
    expect(statuses.toSorted((one, other) => one - other)).toEqual([201, 409]);
  },
);

test('the page says a time was taken meanwhile and shows what is still free', async ({
  page,
  stack,
  invariants,
}) => {
  const { google } = services(stack);
  const { start, zone } = await fillVisit(page, stack, { label: 'late' });
  await calendar(google).busy(start, new Date(Date.parse(start) + MEETING_MS).toISOString());
  await send(page, pastFillTime);
  await expect(page.getByRole('alert')).toContainText('booked a moment ago');
  const taken = page.getByRole('radio', { name: timeOf(start, zone), exact: true });
  await expect(taken).toHaveCount(0);
  const seen = await invariants.take();
  expect(seen.every((problem) => problem.includes('409'))).toBe(true);
});
