import { TZDate } from '@date-fns/tz';
import { button } from '@tonalisk-labs/kit/theme';
import type { InputHTMLAttributes, ReactElement } from 'react';
import { DayPicker, type Modifiers } from 'react-day-picker';

import type { Booking } from '../api/endpoints.ts';
import { BRIEF } from './brief.ts';
import { CALENDAR_CLASSES, FIELD, LINK, TEXT_LINK } from './classes.ts';
import { dayOf, longDayOf, timeOf } from './times.ts';

export const Field = ({
  label,
  error,
  input,
}: {
  readonly label: string;
  readonly error: string | undefined;
  readonly input: InputHTMLAttributes<HTMLInputElement> & { readonly name: string };
}): ReactElement => (
  <p className="flex flex-col gap-1">
    <label htmlFor={input.name} className="font-medium">
      {label}
    </label>
    <input
      {...input}
      id={input.name}
      className={FIELD}
      aria-invalid={error === undefined ? undefined : true}
      aria-describedby={error === undefined ? undefined : `${input.name}-error`}
    />
    {error === undefined ? undefined : (
      <span id={`${input.name}-error`} className="text-sm text-red-800">
        {error}
      </span>
    )}
  </p>
);

/** The visitor's own words: beside a brief, whatever it leaves out. */
export const Message = ({
  brief,
  maxLength,
  error,
}: {
  readonly brief: boolean;
  readonly maxLength: number;
  readonly error: string | undefined;
}): ReactElement => (
  <p className="flex flex-col gap-1">
    <label htmlFor="message" className="font-medium">
      {brief ? 'Anything else? (optional)' : 'What would you like to talk about? (optional)'}
    </label>
    <textarea
      id="message"
      name="message"
      rows={4}
      maxLength={maxLength}
      className={FIELD}
      aria-invalid={error === undefined ? undefined : true}
      aria-describedby={error === undefined ? undefined : 'message-error'}
    />
    {error === undefined ? undefined : (
      <span id="message-error" className="text-sm text-red-800">
        {error}
      </span>
    )}
  </p>
);

export const Brief = (): ReactElement => (
  <fieldset className="flex flex-col gap-4">
    <legend className="mb-2 font-medium">The role</legend>
    {BRIEF.map(({ name, label, required, maxLength, autoComplete }) => (
      <Field
        key={name}
        label={required ? label : `${label} (optional)`}
        error={undefined}
        input={{ name, required, maxLength, autoComplete }}
      />
    ))}
  </fieldset>
);

/** A paid session within the 14-day withdrawal period needs the visitor's express request
 *  (Directive 2011/83/EU, art. 16(m)). */
export const Waiver = ({ error }: { readonly error: string | undefined }): ReactElement => (
  <p className="flex flex-wrap items-start gap-2">
    <input
      id="withdrawalWaiver"
      name="withdrawalWaiver"
      type="checkbox"
      required
      className="mt-1"
      aria-invalid={error === undefined ? undefined : true}
      aria-describedby={error === undefined ? undefined : 'withdrawalWaiver-error'}
    />
    <label htmlFor="withdrawalWaiver" className="min-w-0 flex-1">
      I ask for the session to take place within the 14-day withdrawal period, and I accept that I
      lose my right of withdrawal once it has been given in full (
      <a href="/terms/#withdrawal" className={TEXT_LINK}>
        terms
      </a>
      ).
    </label>
    {error === undefined ? undefined : (
      <span id="withdrawalWaiver-error" className="basis-full text-sm text-red-800">
        {error}
      </span>
    )}
  </p>
);

/** A field people never see and bots fill in: anything in it refuses the booking. */
export const Honeypot = (): ReactElement => (
  <div aria-hidden="true" className="sr-only">
    <label htmlFor="website">Leave this field empty</label>
    <input id="website" name="website" tabIndex={-1} autoComplete="off" />
  </div>
);

export const ZoneGroup = ({
  region,
  zones,
}: {
  readonly region: string;
  readonly zones: readonly string[];
}): ReactElement => (
  <optgroup label={region}>
    {zones.map((name) => (
      <option key={name} value={name}>
        {name.replaceAll('_', ' ')}
      </option>
    ))}
  </optgroup>
);

export const Calendar = ({
  slots,
  days,
  zone,
  day,
  onDay,
}: {
  readonly slots: readonly string[];
  /** The starts by day in `zone`, as the form groups them once for both columns. */
  readonly days: ReadonlyMap<string, readonly string[]>;
  readonly zone: string;
  readonly day: string | undefined;
  readonly onDay: (day: string) => void;
}): ReactElement => {
  const [first] = slots;
  const last = slots.at(-1);
  if (first === undefined || last === undefined) {
    return <p>There are no free times in the coming weeks. Please email hello@marcoferra.com.</p>;
  }
  const [chosen] = day === undefined ? [] : (days.get(day) ?? []);
  const at = (instant: string): TZDate => new TZDate(Date.parse(instant), zone);
  return (
    <DayPicker
      mode="single"
      required
      timeZone={zone}
      weekStartsOn={1}
      defaultMonth={at(first)}
      startMonth={at(first)}
      endMonth={at(last)}
      selected={chosen === undefined ? undefined : at(chosen)}
      onSelect={(date) => {
        onDay(dayOf(date, zone));
      }}
      disabled={(date) => !days.has(dayOf(date, zone))}
      classNames={CALENDAR_CLASSES}
      labels={{ labelDayButton: (date, modifiers) => dayName(date, modifiers, zone) }}
    />
  );
};

/** A day button's name holds its visible number as a word, as the picker's "7th" does not. */
const dayName = (date: Date, { today, selected }: Modifiers, zone: string): string =>
  [
    today === true ? 'Today' : '',
    longDayOf(date.toISOString(), zone),
    selected === true ? 'selected' : '',
  ]
    .filter((part) => part !== '')
    .join(', ');

const TimeOption = ({
  start,
  zone,
}: {
  readonly start: string;
  readonly zone: string;
}): ReactElement => (
  <label className="has-checked:border-primary-700 has-checked:bg-primary-50 flex items-center gap-2 rounded-md border border-slate-400 px-3 py-2">
    <input type="radio" name="start" value={start} required />
    {timeOf(start, zone)}
  </label>
);

export const Times = ({
  times,
  zone,
  minutes,
  error,
}: {
  readonly times: readonly string[];
  readonly zone: string;
  readonly minutes: number;
  readonly error: string | undefined;
}): ReactElement => {
  const [first] = times;
  if (first === undefined) {
    return <p className="text-slate-700">Choose a day to see its free times.</p>;
  }
  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-2 font-medium">
        {longDayOf(first, zone)}, {minutes} minutes
      </legend>
      <div className="flex flex-wrap gap-2">
        {times.map((start) => (
          <TimeOption key={start} start={start} zone={zone} />
        ))}
      </div>
      {error === undefined ? undefined : <span className="text-sm text-red-800">{error}</span>}
    </fieldset>
  );
};

/** The form's button: "Book the meeting", or for a paid type "Pay €40 and book". */
export const Submit = ({
  sending,
  price,
}: {
  readonly sending: boolean;
  readonly price: string | undefined;
}): ReactElement => {
  const label = price === undefined ? 'Book the meeting' : `Pay ${price} and book`;
  return (
    <p>
      <button type="submit" disabled={sending} className={button('primary')}>
        {sending ? 'Booking…' : label}
      </button>
    </p>
  );
};

export const Confirmation = ({
  booked,
  email,
  zone,
}: {
  readonly booked: Booking;
  /** The address it went to, when the page knows it. */
  readonly email: string | undefined;
  readonly zone: string;
}): ReactElement => (
  <section aria-labelledby="done" className="mt-8 flex flex-col gap-4">
    <h2
      id="done"
      tabIndex={-1}
      className="text-2xl font-bold"
      ref={(heading) => {
        if (heading !== null) {
          heading.focus();
        }
      }}
    >
      Check your email
    </h2>
    <p>
      The invitation for {longDayOf(booked.start, zone)} at {timeOf(booked.start, zone)} ({zone}) is
      on its way to {email === undefined ? 'your inbox' : <strong>{email}</strong>}. It carries the
      video link: open it to add the meeting to your calendar.
    </p>
    <p className="flex flex-wrap gap-4">
      <a href={booked.icsUrl} download="invite.ics" className={LINK}>
        Download .ics
      </a>
      <a href={booked.cancelUrl} className={LINK}>
        Cancel this booking
      </a>
    </p>
  </section>
);
