// The happy path: a visitor books on the type's page, which says to check the email once it is
// sent; the invitation is a valid iMIP request for the calendar's own event, and utm_source stays.
import type { Page } from '@playwright/test';
import { calendar, type Coordinates, expect, type Summary, test } from '@tonalisk-labs/kit/testing';

import site from '../../../marcoferra.config.json' with { type: 'json' };
import { basic, services } from '../support/api.ts';
import { icsLines, mailOf } from '../support/mail.ts';
import { MEETING, MEETING_PATH } from '../support/meeting.ts';
import { fillVisit, sendForInvitation, type Visit } from '../support/visit.ts';

// Covers: event_types.duration_minutes, event_types.summary, email.from, email.organizer

const MEETING_MS = MEETING.duration_minutes * 60_000;
/** Where the visitor's link was shared, as the Reddit tool tags it. */
const SOURCE = 'reddit';

interface Booked extends Visit {
  readonly arrived: Summary;
}

/** Books a claimed time from /book/'s card, by a link shared with `utm_source`. */
const bookOnPage = async (page: Page, stack: Coordinates): Promise<Booked> => {
  await page.goto(`/book/?utm_source=${SOURCE}`);
  await page.getByRole('link', { name: `Choose a time for: ${MEETING.title}` }).click();
  await expect(page).toHaveURL(`${MEETING_PATH}?utm_source=${SOURCE}`);
  const visit = await fillVisit(page, stack, { label: 'happy' });
  const arrived = await sendForInvitation(page, stack, visit.guest.email);
  return Object.assign(structuredClone(visit), { arrived });
};

/** A parameter value as RFC 5545 §3.1 writes it: quoted when it holds a colon, semicolon or comma. */
const parameter = (value: string): string => (/[:;,]/u.test(value) ? `"${value}"` : value);

const expectedLines = ({ start, guest }: Booked, video: string): string[] => [
  'METHOD:REQUEST',
  'SEQUENCE:0',
  `DTSTART:${basic(start)}`,
  `DTEND:${basic(Date.parse(start) + MEETING_MS)}`,
  'STATUS:CONFIRMED',
  `SUMMARY:${MEETING.summary}`,
  `ORGANIZER;CN=${parameter('Marco Ferra')}:mailto:${site.email.organizer}`,
  `ATTENDEE;CN=${parameter(guest.name)};PARTSTAT=NEEDS-ACTION;ROLE=REQ-PARTICIPANT;RSVP=TRUE:mailto:${guest.email}`,
  `URL:${video}`,
];

test('a visitor books a time and receives a valid invitation for the calendar event', async ({
  page,
  stack,
}) => {
  const { google, mailpit } = services(stack);
  const booked = await bookOnPage(page, stack);
  expect(booked.arrived.From.Address).toBe(site.email.organizer);
  const { headers, card, invite } = await mailOf(mailpit, booked.arrived);
  expect(headers['Auto-Submitted']).toEqual(['auto-generated']);
  expect(headers['Message-Id']).toEqual([expect.stringMatching(/^<[^@\s]+@marcoferra\.com>$/u)]);
  expect(card, 'the calendar card is the attachment').toBe(invite);
  expect(invite.split('\r\n').filter((line) => Buffer.byteLength(line) > 75)).toEqual([]);

  const lines = icsLines(invite);
  const uid = (lines.find((line) => line.startsWith('UID:')) ?? '').slice('UID:'.length);
  const event = await calendar(google).event(uid.replace(/@google\.com$/u, ''));
  if (event === undefined) {
    throw new Error(`the calendar holds no event ${uid}`);
  }
  expect(event).toMatchObject({
    status: 'confirmed',
    attendees: [{ email: booked.guest.email, displayName: booked.guest.name }],
    extendedProperties: { private: { source: SOURCE } },
  });
  expect(lines).toEqual(expect.arrayContaining(expectedLines(booked, event.hangoutLink ?? '')));
});
