// The measured booking journey, run with KIT_MEASURE=1: the booking page until its calendar
// shows, and a paid return from Checkout until the success page confirms it.
import {
  delayPath,
  edgeIdOf,
  expect,
  measureStep,
  referenceOf,
  test,
  underReference,
} from '@tonalisk-labs/kit/testing';

import { MEETING_PATH } from '../support/meeting.ts';
import { holdOnPage } from '../support/payment.ts';

test('booking: availability', async ({ page }) => {
  await measureStep(page, 'booking: availability', {
    act: async () => page.goto(MEETING_PATH),
    visible: page.getByRole('grid'),
  });
});

/** What production's first read of the free times adds to the harness's: CloudFront to the
 *  region and a cold function, about 1.15 s in all with the slow 4G round trip. */
const COLD_START_MS = 1000;

/** The harness's backend as it answers, and one answering as production's does when cold. */
const ORIGINS = [
  { name: 'booking: first visit on a phone', ms: 0 },
  { name: 'booking: first visit on a phone, cold API', ms: COLD_START_MS },
] as const;

/** A cold first visit until the calendar offers its first free day, on the phone row that gates,
 *  loaded under the mobile reference: Lighthouse's slow 4G with a 4× slower CPU. */
for (const { name, ms } of ORIGINS) {
  test(name, async ({ page, row, stack }) => {
    if (row === undefined) {
      throw new Error('the journeys run on a matrix row');
    }
    const reference = referenceOf(row);
    test.skip(row.form !== 'phone' || reference === undefined, 'the phone row that gates');
    await underReference({ page, row, origin: stack.baseURL }, async () => {
      const pathname = `${stack.apiPrefix ?? ''}/v1/availability`;
      await delayPath(stack.baseURL, edgeIdOf(page.context()), { pathname, ms });
      return measureStep(page, name, {
        act: async () => page.goto(MEETING_PATH),
        visible: page.getByRole('grid').getByRole('button', { disabled: false }).first(),
        network: reference === undefined ? row.network : reference.network,
      });
    });
  });
}

test('booking: pay and confirm', async ({ page, stack }) => {
  await holdOnPage(page, stack, 'measured');
  await expect(page).toHaveURL(/\/checkout\/cs_test_\w+$/u);
  await measureStep(page, 'booking: pay and confirm', {
    act: async () => page.getByRole('button', { name: 'Pay', exact: true }).click(),
    visible: page.getByRole('heading', { name: 'Check your email' }),
  });
});
