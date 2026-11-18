// Opt-in (STRIPE_SANDBOX=1, Chromium desktop): one paid booking and its refund against the
// real Stripe sandbox through `stripe listen`; it fails loudly when the secret or CLI is missing.
import type { Page } from '@playwright/test';
import {
  type Coordinates,
  expect,
  mailbox,
  payAtSandboxCheckout,
  sandboxMissing,
  STRIPE_CHECKOUT,
  STRIPE_HOSTED,
  test,
} from '@tonalisk-labs/kit/testing';

import { services } from '../support/api.ts';
import { holdOnPage, PAID_PATH } from '../support/payment.ts';

const OPTED_IN = process.env['STRIPE_SANDBOX'] === '1';

/** The Chromium desktop row, whose engine Stripe's hosted pages support here. */
const ROWS: ReadonlySet<number> = new Set([1]);

/** Stripe's hosted pages over the real network. */
const LOAD_MS = 60_000;

const DONE = new RegExp(`${PAID_PATH}done/\\?session_id=cs_test_\\w+$`, 'u');

test.use({ actionTimeout: LOAD_MS, timezoneId: 'Europe/Lisbon' });

/** Holds a time, pays for it by card at Stripe, and answers the session the return names. */
const payAtStripe = async (page: Page, stack: Coordinates): Promise<string> => {
  const { guest } = await holdOnPage(page, stack, 'sandbox');
  await expect(page).toHaveURL(STRIPE_CHECKOUT, { timeout: LOAD_MS });
  const arriving = mailbox(services(stack).mailpit).waitFor(guest.email);
  await payAtSandboxCheckout(page, { name: guest.name });
  await expect(page).toHaveURL(DONE, { timeout: LOAD_MS });
  await expect(page.getByRole('heading', { name: 'Check your email' })).toBeVisible();
  await arriving;
  return new URL(page.url()).searchParams.get('session_id') ?? '';
};

/** Stripe told the backend too, through `stripe listen`: an event, bounded here. */
const stripeSaidPaid = async (stack: Coordinates, session: string): Promise<void> => {
  const watch = stack.stripeSandbox;
  if (watch === undefined) {
    throw new Error('the stack runs no Stripe sandbox: STRIPE_SANDBOX=1 starts it');
  }
  const completed = await fetch(`${watch}/events/checkout.session.completed/${session}`, {
    signal: AbortSignal.timeout(LOAD_MS),
  });
  expect(completed.ok, 'Stripe sent checkout.session.completed').toBe(true);
};

test.describe('one paid booking in the real Stripe sandbox', () => {
  test.skip(!OPTED_IN, 'opt-in: set STRIPE_SANDBOX=1 to run against the Stripe sandbox');
  test.skip(
    ({ row }) => row === undefined || !ROWS.has(row.number),
    'runs on the Chromium desktop row',
  );

  // Owns the stack: the sandbox's one account is never paid into by two tests at once.
  test(
    'a visitor pays by card at Stripe, is invited, and cancels with a refund',
    { tag: ['@exclusive'] },
    async ({ page, stack, invariants }, testInfo) => {
      const absent = sandboxMissing();
      if (absent.length > 0) {
        throw new Error(
          `STRIPE_SANDBOX=1, but the sandbox cannot be reached: missing ${absent.join(', ')}`,
        );
      }
      testInfo.setTimeout(10 * 60_000);
      const session = await payAtStripe(page, stack);
      await stripeSaidPaid(stack, session);
      await page.getByRole('link', { name: 'Cancel this booking' }).click();
      await page.getByRole('button', { name: 'Cancel the meeting' }).click();
      const said = page.getByRole('status');
      await expect(said).toContainText('is refunded in full', { timeout: LOAD_MS });
      const problems = await invariants.take();
      const ours = problems.filter((problem) => !STRIPE_HOSTED.test(problem));
      expect(ours, 'the app held every invariant around Stripe').toEqual([]);
    },
  );
});
