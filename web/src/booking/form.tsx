// One native form, with the day picker beside it. A paid type's form asks for the withdrawal
// waiver, whose answer goes on to Stripe Checkout.
import { type ReactElement, useEffect, useState } from 'react';

import type { Availability, Booking } from '../api/endpoints.ts';
import { noteMaxChars, ROLE_TYPE } from './brief.ts';
import { money } from './event-types.ts';
import { Brief, Calendar, Field, Honeypot, Message, Submit, Times, Waiver } from './parts.tsx';
import { book, type Refusal, unanswered } from './requests.ts';
import { byDay } from './times.ts';
import { ZoneChoice } from './zone-choice.tsx';

interface Visit {
  /** The meeting type's slug. */
  readonly type: string;
  readonly availability: Availability;
  readonly zone: string;
  readonly onZone: (zone: string) => void;
  readonly onBooked: (booked: Booking, email: string) => void;
  /** Reads the free times again, after a time was taken meanwhile. */
  readonly onStale: () => void;
}

const NONE = new Map<string, string>();

const DayColumn = ({
  slots,
  days,
  zone,
  day,
  onZone,
  onDay,
}: {
  readonly slots: readonly string[];
  readonly days: ReadonlyMap<string, readonly string[]>;
  readonly zone: string;
  readonly day: string | undefined;
  readonly onZone: (zone: string) => void;
  readonly onDay: (day: string) => void;
}): ReactElement => (
  <section aria-labelledby="day-heading" className="flex flex-col gap-3">
    <h2 id="day-heading" className="text-xl font-semibold">
      Day
    </h2>
    <ZoneChoice zone={zone} onChange={onZone} />
    <Calendar slots={slots} days={days} zone={zone} day={day} onDay={onDay} />
  </section>
);

const Details = ({
  type,
  times,
  zone,
  minutes,
  paid,
  refusal,
}: {
  /** The meeting type's slug: a Role Conversation asks for the brief. */
  readonly type: string;
  readonly times: readonly string[];
  readonly zone: string;
  readonly minutes: number;
  readonly paid: boolean;
  readonly refusal: Refusal | undefined;
}): ReactElement => {
  const errors = refusal === undefined ? NONE : refusal.fields;
  const brief = type === ROLE_TYPE;
  return (
    <>
      <Times times={times} zone={zone} minutes={minutes} error={errors.get('start')} />
      <Field
        label="Name"
        error={errors.get('name')}
        input={{ name: 'name', required: true, autoComplete: 'name' }}
      />
      <Field
        label="Email"
        error={errors.get('email')}
        input={{ name: 'email', required: true, type: 'email', autoComplete: 'email' }}
      />
      {brief ? <Brief /> : undefined}
      <Message brief={brief} maxLength={noteMaxChars(type)} error={errors.get('message')} />
      {paid ? <Waiver error={errors.get('withdrawalWaiver')} /> : undefined}
      <Honeypot />
      {refusal === undefined ? undefined : (
        <p role="alert" className="font-medium text-red-800">
          {refusal.message}
        </p>
      )}
    </>
  );
};

interface Sending {
  readonly refusal: Refusal | undefined;
  readonly sending: boolean;
  readonly submit: (form: FormData) => void;
}

/** Back from Checkout by the browser's own button, the page returns as it was left: sending. */
const useRestored = (setSending: (sending: boolean) => void): void => {
  useEffect(() => {
    const restored = (event: PageTransitionEvent): void => {
      if (event.persisted) {
        setSending(false);
      }
    };
    globalThis.addEventListener('pageshow', restored);
    return (): void => {
      globalThis.removeEventListener('pageshow', restored);
    };
  }, [setSending]);
};

/** One booking attempt per key: a failure keeps its key, so a retry cannot book twice. */
const useAttempt = (
  {
    type,
    zone,
    formToken,
  }: { readonly type: string; readonly zone: string; readonly formToken: string },
  { onBooked, onStale }: Pick<Visit, 'onBooked' | 'onStale'>,
): Sending => {
  const [refusal, setRefusal] = useState<Refusal>();
  const [sending, setSending] = useState(false);
  const [key, setKey] = useState(() => crypto.randomUUID());
  useRestored(setSending);
  const send = async (form: FormData): Promise<void> => {
    const outcome = await book(form, { type, zone, formToken, key }).catch(unanswered);
    if ('held' in outcome) {
      // The page stays busy while the browser leaves for Stripe's own page.
      globalThis.location.assign(outcome.held.checkoutUrl);
      return;
    }
    setSending(false);
    if ('booked' in outcome) {
      onBooked(outcome.booked, outcome.email);
      return;
    }
    setRefusal(outcome);
    if (outcome.settled) {
      setKey(crypto.randomUUID());
    }
    if (outcome.stale) {
      onStale();
    }
  };
  const submit = (form: FormData): void => {
    setSending(true);
    setRefusal(undefined);
    send(form).catch((error: unknown) => {
      setRefusal(unanswered(error));
    });
  };
  return { refusal, sending, submit };
};

export const Form = ({
  type,
  availability,
  zone,
  onZone,
  onBooked,
  onStale,
}: Visit): ReactElement => {
  const [day, setDay] = useState<string>();
  const { slots, formToken, durationMinutes, priceMinor, currency } = availability;
  const paid = priceMinor > 0;
  const { refusal, sending, submit } = useAttempt({ type, zone, formToken }, { onBooked, onStale });
  const days = byDay(slots, zone);
  return (
    <div className="mt-8 flex flex-col gap-8 md:flex-row">
      <DayColumn
        slots={slots}
        days={days}
        zone={zone}
        day={day}
        onZone={(next) => {
          onZone(next);
          setDay(undefined);
        }}
        onDay={setDay}
      />
      <form
        aria-labelledby="details-heading"
        className="flex min-w-0 flex-1 flex-col gap-4"
        onSubmit={(event) => {
          event.preventDefault();
          submit(new FormData(event.currentTarget));
        }}
      >
        <h2 id="details-heading" className="text-xl font-semibold">
          Time and details
        </h2>
        <Details
          type={type}
          times={day === undefined ? [] : (days.get(day) ?? [])}
          zone={zone}
          minutes={durationMinutes}
          paid={paid}
          refusal={refusal}
        />
        <Submit sending={sending} price={paid ? money(priceMinor, currency) : undefined} />
      </form>
    </div>
  );
};
