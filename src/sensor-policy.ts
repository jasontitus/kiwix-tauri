/**
 * Sensor decision policy shared by the reader shell.
 *
 * The reader frame carries a cached `sensorStatus` from the last native call,
 * but the archive can request a sensor at any time. Auto-granting from that
 * cache is only safe while it still matches the live native store and the last
 * explicit choice the user made for this installation. A decision write that
 * failed must never reload the article: the store still holds the old decision,
 * so the reloaded document would replay it (and auto-grant it) even though the
 * user just asked to block.
 */
export type SensorDecision = "notAsked" | "granted" | "denied";

/** What the shell does with an archive sensor request. */
export type SensorRequestDisposition = "allow" | "deny" | "prompt";

/**
 * Decide from the live store decision (`sensor_status`), never the frame's
 * cached status, plus the last explicit choice recorded for the installation.
 * An absent intent means this shell has not changed the decision in this mount;
 * the live store decision is itself the user's earlier choice.
 */
export function sensorRequestDisposition(
  live: SensorDecision,
  intent: SensorDecision | undefined,
): SensorRequestDisposition {
  if (live === "granted" && (intent === undefined || intent === "granted"))
    return "allow";
  if (live === "denied" || intent === "denied") return "deny";
  // `notAsked` in the store (or a reset the user just requested) must prompt.
  return "prompt";
}

export type SensorDecisionWriteOutcome = {
  /** Reload only after the native store accepted the new decision. */
  reload: boolean;
  /** A failed privacy decision is always shown, even with technical details off. */
  blocking: boolean;
};

/**
 * A failed decision write keeps the previous store decision. Reloading would
 * navigate into a document that auto-grants that previous decision, so the
 * failure blocks the reload and is rendered as a blocking error.
 */
export function sensorDecisionWriteOutcome(
  saved: boolean,
): SensorDecisionWriteOutcome {
  return saved
    ? { reload: true, blocking: false }
    : { reload: false, blocking: true };
}
