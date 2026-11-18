// The booking island: nothing says "booked" until the server has sent the invitation, and back
// from Checkout unpaid it first gives the held time up, then shows it free again.
import { type ReactElement, useCallback, useEffect, useState } from 'react';

import type { Availability, Booking as Booked } from '../api/endpoints.ts';
import { Form } from './form.tsx';
import { Confirmation } from './parts.tsx';
import {
  firstAvailability,
  loadAvailability,
  type Refusal,
  release,
  unanswered,
} from './requests.ts';
import { detectedZone } from './times.ts';

/** Checkout's way back names the hold to give up; the address then drops it. */
const releaseOnReturn = async (): Promise<string | undefined> => {
  const query = new URLSearchParams(globalThis.location.search);
  const [id, token] = [query.get('release'), query.get('token')];
  if (id === null || token === null) {
    return undefined;
  }
  globalThis.history.replaceState(undefined, '', globalThis.location.pathname);
  const released = await release(id, token).catch(() => false);
  return released
    ? 'Payment cancelled: nothing was charged, and the time you chose is free again.'
    : 'Payment cancelled: nothing was charged. The time you chose is freed once its hold ends.';
};

/** The first form token stays: it dates the visit, not the latest read. */
const keepToken =
  (fresh: Availability | Refusal) =>
  (current: Availability | Refusal | undefined): Availability | Refusal =>
    current !== undefined && 'formToken' in current && 'formToken' in fresh
      ? Object.assign(structuredClone(fresh), { formToken: current.formToken })
      : fresh;

interface Visit {
  readonly availability: Availability | Refusal | undefined;
  /** What the way back from Checkout came to, said once. */
  readonly notice: string | undefined;
  readonly load: () => void;
}

/** The free times, read once any hold the visitor came back from is given up. */
const useAvailability = (type: string): Visit => {
  const [availability, setAvailability] = useState<Availability | Refusal>();
  const [notice, setNotice] = useState<string>();
  const show = useCallback((reading: Promise<Availability | Refusal>): void => {
    reading.then(
      (fresh) => {
        setAvailability(keepToken(fresh));
      },
      (error: unknown) => {
        setAvailability(unanswered(error));
      },
    );
  }, []);
  const load = useCallback((): void => {
    show(loadAvailability(type));
  }, [show, type]);
  useEffect(() => {
    const first = (): void => {
      show(firstAvailability(type));
    };
    releaseOnReturn().then((said) => {
      setNotice(said);
      first();
    }, first);
  }, [show, type]);
  return { availability, notice, load };
};

const Notice = ({ said }: { readonly said: string | undefined }): ReactElement | undefined =>
  said === undefined ? undefined : <output className="mt-8 block font-medium">{said}</output>;

const Booking = ({ type }: { readonly type: string }): ReactElement => {
  const { availability, notice, load } = useAvailability(type);
  const [zone, setZone] = useState(detectedZone);
  const [booked, setBooked] = useState<{ booked: Booked; email: string }>();

  if (booked !== undefined) {
    return <Confirmation booked={booked.booked} email={booked.email} zone={zone} />;
  }
  if (availability === undefined) {
    return (
      <>
        <Notice said={notice} />
        <p className="mt-8 text-slate-700">Loading the free times…</p>
      </>
    );
  }
  if (!('formToken' in availability)) {
    return (
      <p role="alert" className="mt-8">
        {availability.message}
      </p>
    );
  }
  return (
    <>
      <Notice said={notice} />
      <Form
        type={type}
        availability={availability}
        zone={zone}
        onZone={setZone}
        onBooked={(done, email) => {
          setBooked({ booked: done, email });
        }}
        onStale={load}
      />
    </>
  );
};

export default Booking;
