// The meeting type the journeys book: the first the configuration lists, and its page.
import { type EventType, eventTypes } from '../../src/booking/event-types.ts';

const [first] = eventTypes();
if (first === undefined) {
  throw new Error('marcoferra.config.json lists no meeting type');
}

export const MEETING: EventType = first;

export const MEETING_PATH = `/book/${MEETING.slug}/`;
