// The Role Conversation: the brief heads the message, so it reaches the calendar event but
// never outgoing mail; a brief and a note at their longest fit the message's limit.
import type { Locator, Page } from '@playwright/test';
import { apiTest, calendar, type Coordinates, expect, test } from '@tonalisk-labs/kit/testing';

import {
  BRIEF,
  MESSAGE_MAX_CHARS,
  messageOf,
  noteMaxChars,
  ROLE_TYPE,
} from '../../src/booking/brief.ts';
import { eventTypes } from '../../src/booking/event-types.ts';
import { book, callerFor, merge, readyBooking, services } from '../support/api.ts';
import { icsLines, type Mail, mailOf } from '../support/mail.ts';
import { fillVisit, sendForInvitation } from '../support/visit.ts';

// Covers: limits.message_max_chars

const role = eventTypes().find(({ slug }) => slug === ROLE_TYPE);
if (role === undefined) {
  throw new Error(`marcoferra.config.json lists no ${ROLE_TYPE}`);
}
const ROLE = role;

/** A brief as a recruiter gives it, by each field's label. */
const ANSWERS: readonly (readonly [string, string])[] = [
  ['Company', 'Acme Analytics'],
  ['Role title', 'Head of Delivery'],
  ['Level', 'Director'],
  ['Location or remote', 'Remote, Europe'],
  ['Compensation range (optional)', 'EUR 150k to 180k'],
];
const NOTE = 'Eight teams, and a platform migration this year.';

/** The message the booking carries: the brief's lines, then the note. */
const MESSAGE = [
  'Company: Acme Analytics',
  'Role title: Head of Delivery',
  'Level: Director',
  'Location or remote: Remote, Europe',
  'Compensation range: EUR 150k to 180k',
  '',
  NOTE,
].join('\n');

/** Fills the brief one field after another, as a person does. */
const fillBrief = async (
  brief: Locator,
  answers: readonly (readonly [string, string])[],
): Promise<void> => {
  const [first, ...rest] = answers;
  if (first === undefined) {
    return;
  }
  await brief.getByLabel(first[0], { exact: true }).fill(first[1]);
  await fillBrief(brief, rest);
};

interface Booked {
  readonly guest: { readonly name: string; readonly email: string };
  readonly mail: Mail;
}

/** Books a claimed time from Work with me's Roles door, with the brief and a note. */
const bookWithBrief = async (page: Page, stack: Coordinates): Promise<Booked> => {
  await page.goto('/work-with-me/');
  await page.getByRole('link', { name: `Choose a time for: ${ROLE.title}` }).click();
  await expect(page).toHaveURL(`/book/${ROLE_TYPE}/`);
  const { guest } = await fillVisit(page, stack, { label: 'role', kind: ROLE });
  const brief = page.getByRole('group', { name: 'The role' });
  // Every answer but the range is required.
  await expect(brief.locator('input[required]')).toHaveCount(BRIEF.length - 1);
  await fillBrief(brief, ANSWERS);
  const note = page.getByLabel('Anything else? (optional)', { exact: true });
  await expect(note).toHaveAttribute('maxlength', String(noteMaxChars(ROLE_TYPE)));
  await note.fill(NOTE);
  const arrived = await sendForInvitation(page, stack, guest.email);
  return { guest, mail: await mailOf(services(stack).mailpit, arrived) };
};

test('a recruiter books a Role Conversation, and the brief reaches the calendar alone', async ({
  page,
  stack,
}) => {
  const { guest, mail } = await bookWithBrief(page, stack);
  const sent = [mail.text, mail.html, mail.card, mail.invite].map((body) =>
    body.replaceAll('\r\n ', ''),
  );
  for (const answer of ['Acme Analytics', 'Head of Delivery', NOTE.slice(0, 20)]) {
    expect(sent.filter((body) => body.includes(answer))).toEqual([]);
  }
  const lines = icsLines(mail.invite);
  expect(lines).toContain(`SUMMARY:${ROLE.summary}`);
  const uid = (lines.find((line) => line.startsWith('UID:')) ?? '').slice('UID:'.length);
  const event = await calendar(services(stack).google).event(uid.replace(/@google\.com$/u, ''));
  expect(event).toMatchObject({
    summary: ROLE.summary,
    description: `Booked on marcoferra.com by ${guest.name} <${guest.email}>.\n\n${MESSAGE}`,
    extendedProperties: { private: { type: ROLE_TYPE } },
  });
});

apiTest(
  'a brief and a note at their longest fit the message the booking accepts',
  { tag: '@api' },
  async ({ stack }) => {
    const form = new FormData();
    for (const { name, maxLength } of BRIEF) {
      form.set(name, 'x'.repeat(maxLength));
    }
    form.set('message', 'y'.repeat(noteMaxChars(ROLE_TYPE)));
    const message = messageOf(form, ROLE_TYPE);
    expect(message.length).toBe(MESSAGE_MAX_CHARS);
    const caller = callerFor(stack);
    const ready = await readyBooking(caller, 'role-longest', ROLE);
    expect(await book(caller, merge(ready, { message }))).toMatchObject({ status: 201 });
  },
);
