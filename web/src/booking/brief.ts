// The brief a Role Conversation asks for. The booking API keeps one message, so the brief goes
// at its head as labelled lines, and the message's limit (marcoferra.config.json) covers both.
import site from '../../../marcoferra.config.json' with { type: 'json' };

/** The meeting type whose form asks for the brief. */
export const ROLE_TYPE = 'role-conversation';

export interface BriefField {
  /** The form field's name, which the request never sends as its own member. */
  readonly name: string;
  /** The field's label, and the line's in the message. */
  readonly label: string;
  readonly required: boolean;
  readonly maxLength: number;
  readonly autoComplete: string;
}

export const BRIEF: readonly BriefField[] = [
  {
    name: 'company',
    label: 'Company',
    required: true,
    maxLength: 100,
    autoComplete: 'organization',
  },
  { name: 'roleTitle', label: 'Role title', required: true, maxLength: 100, autoComplete: 'off' },
  { name: 'level', label: 'Level', required: true, maxLength: 60, autoComplete: 'off' },
  {
    name: 'workplace',
    label: 'Location or remote',
    required: true,
    maxLength: 100,
    autoComplete: 'off',
  },
  {
    name: 'range',
    label: 'Compensation range',
    required: false,
    maxLength: 60,
    autoComplete: 'off',
  },
];

/** The longest message the API accepts, in characters. */
export const MESSAGE_MAX_CHARS = site.limits.message_max_chars;

/** The brief at its longest: every line filled to its limit, and the blank line after it. */
const BRIEF_MAX_CHARS = BRIEF.reduce(
  (total, { label, maxLength }) => total + `${label}: `.length + maxLength + 1,
  1,
);

/** How long the visitor's own words may be, beside the brief when the type asks for one. */
export const noteMaxChars = (type: string): number =>
  type === ROLE_TYPE ? MESSAGE_MAX_CHARS - BRIEF_MAX_CHARS : MESSAGE_MAX_CHARS;

const text = (form: FormData, name: string): string => {
  const value = form.get(name);
  return typeof value === 'string' ? value.trim() : '';
};

/** For a Role Conversation, the brief's filled lines, a blank line, then the visitor's words. */
export const messageOf = (form: FormData, type: string): string => {
  const note = text(form, 'message');
  if (type !== ROLE_TYPE) {
    return note;
  }
  const lines = BRIEF.flatMap(({ name, label }) => {
    const answer = text(form, name);
    return answer === '' ? [] : [`${label}: ${answer}`];
  });
  return [lines.join('\n'), note].filter((part) => part !== '').join('\n\n');
};
