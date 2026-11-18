// Cancelling: the booking's link cancels it, METHOD:CANCEL arrives for the same UID, the time
// is free again, and a second cancel is refused as gone.
import { expect, test } from '@tonalisk-labs/kit/testing';

import {
  book,
  callerFor,
  cancel,
  freeStarts,
  ics,
  readyBooking,
  services,
  tokenOf,
} from '../support/api.ts';
import { icsLines, invitationThenNext, mailOf, uidOf } from '../support/mail.ts';

test('a visitor cancels from the link and the invitation is withdrawn', async ({ page, stack }) => {
  const { mailpit } = services(stack);
  const caller = callerFor(stack);
  const request = await readyBooking(caller, 'cancel');
  const booked = await book(caller, request);
  if (booked.status !== 201) {
    throw new Error(`the booking answered ${String(booked.status)}`);
  }
  const { invitation, next } = await invitationThenNext(mailpit, request.email);
  await page.goto(booked.data.cancelUrl);
  await page.getByRole('button', { name: 'Cancel the meeting' }).click();
  await expect(page.getByRole('status')).toHaveText(/The meeting is cancelled/u);

  const [requested, cancelled] = await Promise.all([mailOf(mailpit, invitation), next]);
  expect(uidOf(cancelled.invite)).toBe(uidOf(requested.invite));
  expect(icsLines(cancelled.invite)).toEqual(
    expect.arrayContaining(['METHOD:CANCEL', 'SEQUENCE:1', 'STATUS:CANCELLED']),
  );

  const { id, cancelUrl, icsUrl } = booked.data;
  const [again, gone] = await Promise.all([
    cancel(caller, cancelUrl),
    ics(caller, id, tokenOf(icsUrl)),
  ]);
  expect([again.status, gone.status]).toEqual([410, 410]);
  expect(await freeStarts(stack), 'the time is free again').toContain(request.start);
});
