// The free type: no payment or waiver, one upcoming meeting per address, which a paid booking
// does not count against; the cancel page notes refunds only for a paid type its link names.
import { apiTest, expect, test } from '@tonalisk-labs/kit/testing';

import { book, callerFor, merge, readyBooking } from '../support/api.ts';
import { MEETING, MEETING_PATH } from '../support/meeting.ts';
import { bookPaid } from '../support/payment.ts';
import { calendarShown } from '../support/ui.ts';

// Covers: event_types.payment_methods, event_types.slug

apiTest(
  'the free type books at once, beside a paid booking of the same address',
  { tag: '@api' },
  async ({ stack }) => {
    const caller = callerFor(stack);
    const paid = await bookPaid(caller, 'free-beside-paid');
    const free = await readyBooking(caller, 'free-beside-paid');
    const same = { name: paid.request.name, email: paid.request.email };
    const first = await book(caller, merge(free, same));
    expect(first.status, 'a paid booking does not count against the free one').toBe(201);
    const second = await readyBooking(caller, 'free-twice');
    expect(await book(caller, merge(second, same))).toMatchObject({
      status: 409,
      data: { type: 'tag:marcoferra.com,2026:already-booked' },
    });
  },
);

test('the free page asks for no payment and no waiver', async ({ page }) => {
  expect(MEETING.payment_methods).toEqual([]);
  await page.goto('/book/');
  await expect(page.getByRole('article', { name: MEETING.title })).toContainText('Free');
  await page.goto(MEETING_PATH);
  await calendarShown(page);
  await expect(page.getByRole('button', { name: 'Book the meeting' })).toBeVisible();
  await expect(page.getByRole('checkbox')).toHaveCount(0);
});

test('the cancel page notes refunds by the type its link names, neutrally for an old link', async ({
  page,
  stack,
}) => {
  const caller = callerFor(stack);
  const free = await book(caller, await readyBooking(caller, 'free-cancel-page'));
  if (free.status !== 201) {
    throw new Error(`the booking answered ${String(free.status)}`);
  }
  expect(free.data.cancelUrl).toContain(`type=${MEETING.slug}`);
  await page.goto(free.data.cancelUrl);
  await expect(page.getByRole('button', { name: 'Cancel the meeting' })).toBeVisible();
  await expect(page.getByText('This meeting is paid for.')).toBeHidden();
  await expect(page.getByText('If you paid for this meeting')).toBeHidden();
  // A link from before types were named: no type, so no claim either way.
  await page.goto('/book/cancel/?id=mfold&token=old');
  await expect(page.getByText('If you paid for this meeting')).toBeVisible();
  await expect(page.getByText('This meeting is paid for.')).toBeHidden();
});
