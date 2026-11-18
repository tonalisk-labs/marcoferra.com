// Back from Stripe Checkout paid: the server confirms the payment with Stripe and sends the
// invitation, as Stripe's webhook would; whichever comes first books it, once.
import { type ReactElement, useEffect, useState } from 'react';

import { Confirmation } from './parts.tsx';
import { type Confirmed, confirm, failureOf } from './requests.ts';
import { detectedZone } from './times.ts';

const Heading = ({ children }: { readonly children: string }): ReactElement => (
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
    {children}
  </h2>
);

const Done = (): ReactElement => {
  // The session Stripe's success URL names, if any.
  const session = new URLSearchParams(globalThis.location.search).get('session_id');
  const [outcome, setOutcome] = useState<Confirmed>();
  useEffect(() => {
    if (session !== null) {
      confirm(session).then(setOutcome, (error: unknown) => {
        setOutcome({ refusal: failureOf(error) });
      });
    }
  }, [session]);

  if (session === null) {
    return (
      <p role="alert" className="mt-8">
        There is no payment to confirm here.
      </p>
    );
  }
  if (outcome === undefined) {
    return <p className="mt-8 text-slate-700">Confirming your payment…</p>;
  }
  if ('booked' in outcome) {
    return <Confirmation booked={outcome.booked} email={undefined} zone={detectedZone()} />;
  }
  if ('pending' in outcome) {
    return (
      <section aria-labelledby="done" className="mt-8 flex flex-col gap-4">
        <Heading>Payment pending</Heading>
        <p>{outcome.pending}</p>
      </section>
    );
  }
  return (
    <p role="alert" className="mt-8">
      {outcome.refusal}
    </p>
  );
};

export default Done;
