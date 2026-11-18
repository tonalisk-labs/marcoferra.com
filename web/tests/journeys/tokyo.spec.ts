// A visitor in Tokyo: the page shows and groups every time as Tokyo reads it, the email
// states it there too, and changing the zone moves them.
import { expect, mailbox, test } from '@tonalisk-labs/kit/testing';

import { longDayOf, timeOf } from '../../src/booking/times.ts';
import { claimStart, freeStarts, pastFillTime, services, visitor } from '../support/api.ts';
import { MEETING_PATH } from '../support/meeting.ts';
import { chooseDay, chooseStart, fillDetails, send } from '../support/ui.ts';

// Covers: booking.host_time_zone

const TOKYO = 'Asia/Tokyo';
const LISBON = 'Europe/Lisbon';

test.use({ timezoneId: TOKYO });

test('a visitor in Tokyo sees and is sent Tokyo times', async ({ page, stack }) => {
  const inbox = mailbox(services(stack).mailpit);
  const start = await claimStart(stack);
  const guest = visitor('tokyo');
  await page.goto(MEETING_PATH);
  await expect(page.getByText(`Times are shown in ${TOKYO}.`)).toBeVisible();
  await chooseStart(page, start, TOKYO);
  await expect(page.getByRole('group', { name: longDayOf(start, TOKYO) })).toBeVisible();
  await fillDetails(page, guest.name, guest.email);
  const arriving = inbox.waitFor(guest.email);
  await send(page, pastFillTime);
  await expect(page.getByRole('main')).toContainText(`at ${timeOf(start, TOKYO)} (${TOKYO})`);
  const arrived = await arriving;
  const { Text: text } = await inbox.read(arrived.ID);
  expect(text).toContain(`${timeOf(start, TOKYO)} to`);
  expect(text).toContain(`(${TOKYO})`);
});

test('changing the zone shows the same times in the zone chosen', async ({ page, stack }) => {
  const [start] = await freeStarts(stack);
  if (start === undefined) {
    throw new Error('the calendar offers no time');
  }
  await page.goto(MEETING_PATH);
  await page.getByRole('button', { name: 'Change' }).click();
  await page.getByLabel('Time zone').selectOption(LISBON);
  await chooseDay(page, start, LISBON);
  const time = page.getByRole('radio', { name: timeOf(start, LISBON), exact: true });
  await expect(time).toBeVisible();
});
