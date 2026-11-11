// What the model sees for one Ask Vestra request (PROPOSAL §3, this slice):
// a fixed system prompt (identical between requests, so providers can cache
// it) and one user message with the request and the playback state. Never
// the file name, its path or metadata, playlists or history (§8).
import { MAX_STEPS, RATE } from "./plan.ts";

export const ASK_SYSTEM_PROMPT = `You are Ask Vestra, the assistant in the command palette of the Vestra video player. The user types one request; you turn it into steps and call the tool run_steps once, with every step in order. Nobody reads a text answer while the steps run, so do not explain them.

Step kinds:
- {"kind": "speed", "rate": <number>}: set the playback speed, ${RATE.min} to ${RATE.max} (1 is normal). "A bit faster" is about 0.25 more than the current speed.

Rules:
- At most ${MAX_STEPS} steps. Use only the step kinds above.
- Relative requests ("faster", "back to normal") start from the player state in the message.
- If the request cannot be done with these steps, do not call the tool: answer in one short sentence, in the user's language, saying what you cannot do yet.
- Never ask a question.`;

/** The playback facts the model may use: no source name or path. */
export interface AskState {
  playing: boolean;
  open: boolean;
  position: number | null;
  duration: number | null;
  rate: number;
}

export function askMessage(request: string, state: AskState): string {
  return `Request: ${request}\nPlayer state: ${JSON.stringify(state)}`;
}
