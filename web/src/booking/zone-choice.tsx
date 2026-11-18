import { type ReactElement, useState } from 'react';

import { FIELD, LINK } from './classes.ts';
import { ZoneGroup } from './parts.tsx';
import { zonesByRegion } from './times.ts';

export const ZoneChoice = ({
  zone,
  onChange,
}: {
  readonly zone: string;
  readonly onChange: (zone: string) => void;
}): ReactElement => {
  const [open, setOpen] = useState(false);
  if (!open) {
    return (
      <p className="text-slate-700">
        Times are shown in <strong>{zone}</strong>.{' '}
        <button
          type="button"
          className={LINK}
          onClick={() => {
            setOpen(true);
          }}
        >
          Change
        </button>
      </p>
    );
  }
  return (
    <p className="flex flex-col gap-1">
      <label htmlFor="zone" className="font-medium">
        Time zone
      </label>
      <select
        id="zone"
        className={FIELD}
        value={zone}
        onChange={(event) => {
          onChange(event.target.value);
        }}
      >
        {zonesByRegion(zone).map(([region, zones]) => (
          <ZoneGroup key={region} region={region} zones={zones} />
        ))}
      </select>
    </p>
  );
};
