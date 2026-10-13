import { MagnifyingGlass } from "@phosphor-icons/react";
import iwaksMark from "./assets/iwaks-mark.png";

/**
 * The two "nothing to show" states of the library, kept apart on purpose.
 *
 * They are separate components because telling them apart *is* the fix (issue
 * #04): for a user with 20k tracks who mistyped a search, showing "Your music
 * lives here — point Iwaks at a folder" claims their library vanished. Anything
 * that puts both states in one component with a boolean flag is one edit away
 * from regressing.
 */

export interface NoResultsProps {
  /** The text typed into the search box, trimmed. Empty when only filters are on. */
  query: string;
  /** Whether a genre/decade filter is narrowing the view (issue #20). */
  filtered: boolean;
  onClearQuery: () => void;
  onClearFilters: () => void;
}

/**
 * "A search or filter matched nothing." Says which one, and offers the way back.
 */
export function NoResultsState({
  query,
  filtered,
  onClearQuery,
  onClearFilters,
}: NoResultsProps) {
  const hasQuery = query.trim() !== "";
  return (
    <div className="empty no-results">
      <MagnifyingGlass className="empty-mark-icon" size={34} aria-hidden="true" />
      <h2>
        {hasQuery ? `No results for “${query.trim()}”` : "Nothing matches those filters"}
      </h2>
      <p>
        {filtered
          ? "No tracks in your library match this genre and year."
          : "Check the spelling, or try a different song, artist, or album."}
      </p>
      <div className="empty-actions">
        {hasQuery && (
          <button className="btn-ghost" onClick={onClearQuery}>
            Clear search
          </button>
        )}
        {filtered && (
          <button className="btn-ghost" onClick={onClearFilters}>
            Clear filters
          </button>
        )}
      </div>
    </div>
  );
}

export interface OnboardingProps {
  /** A scan is running: the copy changes from an invitation to a progress note. */
  scanning: boolean;
  onScan: () => void;
}

/**
 * "The library really is empty." Only ever shown with no query and no filter.
 */
export function OnboardingState({ scanning, onScan }: OnboardingProps) {
  return (
    <div className="empty">
      <img className="empty-mark" src={iwaksMark} alt="" />
      <h2>{scanning ? "Building your library…" : "Your music lives here"}</h2>
      <p>
        {scanning
          ? "Keep this window open while Iwaks finds your tracks."
          : "Point Iwaks at a folder on this PC and it will scan hi-res & lossless formats."}
      </p>
      <button className="btn-primary" onClick={onScan} disabled={scanning}>
        {scanning ? "Scanning…" : "Scan a folder"}
      </button>
    </div>
  );
}