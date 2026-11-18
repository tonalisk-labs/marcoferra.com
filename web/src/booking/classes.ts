// Tailwind classes more than one piece uses, and the day picker's. A control's edge is slate-400,
// a divider's slate-200, and the looks every site shares are kit's.

export const FIELD = 'rounded-md border border-slate-400 px-3 py-2 aria-invalid:border-red-700';

/** A link within a sentence; one that stands alone (LINK) is a finger's height on a touch screen. */
export const TEXT_LINK = 'underline underline-offset-4';
export const LINK = `${TEXT_LINK} pointer-coarse:inline-block pointer-coarse:py-3`;

export const CALENDAR_CLASSES = {
  root: 'relative w-fit',
  month: 'flex flex-col gap-2',
  month_caption: 'flex h-10 items-center px-2 font-semibold',
  nav: 'absolute end-0 top-0 flex gap-1',
  button_previous: 'grid size-10 place-items-center rounded-full aria-disabled:opacity-40',
  button_next: 'grid size-10 place-items-center rounded-full aria-disabled:opacity-40',
  chevron: 'size-5 fill-current',
  month_grid: 'border-collapse',
  weekday: 'size-10 text-sm font-medium text-slate-600',
  day: 'p-0.5 text-center',
  day_button:
    'size-10 rounded-full font-medium enabled:hover:bg-primary-50 disabled:font-normal disabled:text-slate-400',
  selected: '*:bg-primary-700 *:text-white',
  today: '*:underline',
  outside: 'invisible',
};
