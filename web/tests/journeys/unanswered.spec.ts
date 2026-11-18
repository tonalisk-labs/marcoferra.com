// No answer from the API: a request the network drops, or one something else answers, says
// which in its own words; the free times say so too, and the same attempt then books.
import { expect, test } from '@tonalisk-labs/kit/testing';

import { pastFillTime } from '../support/api.ts';
import { MEETING_PATH } from '../support/meeting.ts';
import { send } from '../support/ui.ts';
import { fillVisit, sendForInvitation } from '../support/visit.ts';

const OFFLINE = 'Your connection seems to be down';
const UNAVAILABLE = "The booking service isn't answering right now.";

test('a booking with no answer says why, and the same attempt then books', async ({
  page,
  stack,
  invariants,
}) => {
  const { guest } = await fillVisit(page, stack, { label: 'unanswered' });
  await page.route('**/api/v1/bookings', async (route) => route.abort('internetdisconnected'));
  await send(page, pastFillTime);
  await expect(page.getByRole('alert')).toContainText(OFFLINE);
  await page.unrouteAll();
  await page.route('**/api/v1/bookings', async (route) =>
    route.fulfill({ status: 502, contentType: 'text/html', body: '<h1>Bad gateway</h1>' }),
  );
  await send(page, pastFillTime);
  await expect(page.getByRole('alert')).toContainText(UNAVAILABLE);
  await page.unrouteAll();
  await sendForInvitation(page, stack, guest.email);
  const seen = await invariants.take();
  expect(
    seen.every((problem) => problem.includes('/v1/bookings')),
    seen.join('\n'),
  ).toBe(true);
});

test('free times the network drops say so', async ({ page, invariants }) => {
  await page.route('**/api/v1/availability**', async (route) =>
    route.abort('internetdisconnected'),
  );
  await page.goto(MEETING_PATH);
  await expect(page.getByRole('alert')).toContainText(OFFLINE);
  const seen = await invariants.take();
  expect(
    seen.every((problem) => problem.includes('/v1/availability')),
    seen.join('\n'),
  ).toBe(true);
});
