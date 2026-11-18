// The download: "Download .ics" gives the invitation the email carried, byte for byte, so
// adding either never makes two events.
import { buffer } from 'node:stream/consumers';

import { expect, test } from '@tonalisk-labs/kit/testing';

import { services } from '../support/api.ts';
import { mailOf } from '../support/mail.ts';
import { fillVisit, sendForInvitation } from '../support/visit.ts';

test('the downloaded invitation is the emailed one', async ({ page, stack }) => {
  const { guest } = await fillVisit(page, stack, { label: 'download' });
  const arrived = await sendForInvitation(page, stack, guest.email);
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByRole('link', { name: 'Download .ics' }).click(),
  ]);
  const bytes = await buffer(await download.createReadStream());
  const { invite } = await mailOf(services(stack).mailpit, arrived);
  expect(bytes.toString('utf8')).toBe(invite);
});
