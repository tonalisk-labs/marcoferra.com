// A booking's email as Mailpit parsed it. The backend sends mixed(alternative(text, HTML,
// calendar card), invite.ics), so the card is part 1.3 and the attachment part 2.
import { mailbox, type Summary } from '@tonalisk-labs/kit/testing';

export interface Mail {
  readonly headers: Readonly<Record<string, readonly string[]>>;
  readonly text: string;
  readonly html: string;
  readonly card: string;
  readonly invite: string;
}

const isHeaders = (value: unknown): value is Record<string, string[]> =>
  typeof value === 'object' &&
  value !== null &&
  Object.values(value).every((values) => Array.isArray(values));

const fetched = async (url: string): Promise<Response> => {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`Mailpit answered ${String(response.status)} for ${url}`);
  }
  return response;
};

const headersAt = async (url: string): Promise<Record<string, string[]>> => {
  const response = await fetched(url);
  const body: unknown = await response.json();
  if (!isHeaders(body)) {
    throw new TypeError(`Mailpit returned no headers at ${url}`);
  }
  return body;
};

const textAt = async (url: string): Promise<string> => {
  const response = await fetched(url);
  return response.text();
};

/** The message a list named, which must be there. */
export const mailOf = async (mailpit: string, message: Summary | undefined): Promise<Mail> => {
  if (message === undefined) {
    throw new Error('a message is missing');
  }
  const url = `${mailpit}/api/v1/message/${encodeURIComponent(message.ID)}`;
  const [{ Text, HTML }, headers, card, invite] = await Promise.all([
    mailbox(mailpit).read(message.ID),
    headersAt(`${url}/headers`),
    textAt(`${url}/part/1.3`),
    textAt(`${url}/part/2`),
  ]);
  return { headers, text: Text, html: HTML, card, invite };
};

/** An iCalendar body unfolded into its property lines. */
export const icsLines = (ics: string): string[] => ics.replaceAll('\r\n ', '').split('\r\n');

export const uidOf = (ics: string): string | undefined =>
  icsLines(ics).find((line) => line.startsWith('UID:'));

/** The invitation to `email`, once it has arrived, and the next message to it, still to come:
 *  Mailpit's order is not the order they were sent in. */
export const invitationThenNext = async (
  mailpit: string,
  email: string,
): Promise<{ readonly invitation: Summary; readonly next: Promise<Mail> }> => {
  const inbox = mailbox(mailpit);
  const [invitation] = await inbox.waitForCount(email, 1);
  if (invitation === undefined) {
    throw new Error(`no invitation to ${email} arrived`);
  }
  const next = inbox.waitForCount(email, 2).then(async (both) =>
    mailOf(
      mailpit,
      both.find(({ ID }) => ID !== invitation.ID),
    ),
  );
  return { invitation, next };
};
