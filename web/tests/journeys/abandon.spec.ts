// Abandonment: back from Checkout unpaid, the page frees the hold at once and says nothing was
// charged; the event is gone, the session expired, the webhook finds nothing to do, nothing sent.
import { calendar, expect, test } from '@tonalisk-labs/kit/testing';

import { timeOf } from '../../src/booking/times.ts';
import { sentTo, services } from '../support/api.ts';
import { fakeStripe, holdOnPage, PAID_PATH, sessionOf } from '../support/payment.ts';
import { chooseDay } from '../support/ui.ts';

const RELEASED = 'Payment cancelled: nothing was charged, and the time you chose is free again.';

test('a visitor who leaves Checkout unpaid finds the time free again', async ({ page, stack }) => {
  const { google, mailpit } = services(stack);
  const stripe = fakeStripe(stack);
  const { start, zone, guest } = await holdOnPage(page, stack, 'abandon');
  await expect(page).toHaveURL(/\/checkout\/cs_test_\w+$/u);
  const session = sessionOf(page.url());
  const { client_reference_id: id } = await stripe.session(session);

  await page.getByRole('link', { name: 'Back' }).click();
  // The day picker's month caption is a status too: the notice is the one saying this.
  await expect(page.getByRole('status').filter({ hasText: 'Payment cancelled' })).toHaveText(
    RELEASED,
  );
  await expect(page, 'the way back leaves no token in the address').toHaveURL(PAID_PATH);
  await chooseDay(page, start, zone);
  await expect(page.getByRole('radio', { name: timeOf(start, zone), exact: true })).toBeVisible();

  expect(await calendar(google).event(String(id))).toMatchObject({ status: 'cancelled' });
  expect(await stripe.session(session)).toMatchObject({ status: 'expired' });
  expect(await stripe.delivered('checkout.session.expired', session)).toEqual([200]);
  expect(await sentTo(mailpit, guest.email)).toEqual([]);
});
