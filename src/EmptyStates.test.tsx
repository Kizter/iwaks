import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NoResultsState, OnboardingState } from "./EmptyStates";

/**
 * Issue #04: "a search that matched nothing" and "the library is empty" are
 * different facts, and the app used to show the onboarding copy for both. These
 * tests exist to make that impossible to reintroduce quietly — the claim is
 * about what the user reads, so it is asserted on the rendered text.
 */
describe("NoResultsState", () => {
  const handlers = () => ({
    onClearQuery: vi.fn(),
    onClearFilters: vi.fn(),
  });

  it("names the query that matched nothing", () => {
    render(<NoResultsState query="beatels" filtered={false} {...handlers()} />);
    expect(screen.getByText(/No results for/)).toHaveTextContent('“beatels”');
    expect(
      screen.queryByText(/Your music lives here/),
      "never claims the library is empty",
    ).not.toBeInTheDocument();
  });

  it("offers the way back out, and it works", async () => {
    const user = userEvent.setup();
    const onClearQuery = vi.fn();
    const onClearFilters = vi.fn();
    render(
      <NoResultsState
        query="beatels"
        filtered={false}
        onClearQuery={onClearQuery}
        onClearFilters={onClearFilters}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Clear search" }));
    expect(onClearQuery, "clearing the search is what the button promises").toHaveBeenCalledOnce();
    expect(onClearFilters).not.toHaveBeenCalled();
  });

  it("distinguishes a filter that matched nothing from a typo", () => {
    render(<NoResultsState query="" filtered={true} {...handlers()} />);
    expect(screen.getByText("Nothing matches those filters")).toBeInTheDocument();
    expect(screen.getByText(/match this genre and year/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Clear filters" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Clear search" }),
      "nothing to clear when no search was typed",
    ).not.toBeInTheDocument();
  });

  it("offers both escapes when both narrowed the view", () => {
    render(<NoResultsState query="jazz" filtered={true} {...handlers()} />);
    expect(screen.getByRole("button", { name: "Clear search" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Clear filters" })).toBeInTheDocument();
  });

  it("trims the query it echoes back", () => {
    render(<NoResultsState query="  beatels  " filtered={false} {...handlers()} />);
    expect(screen.getByText(/No results for/)).toHaveTextContent("“beatels”");
    expect(screen.getByText(/No results for/)).not.toHaveTextContent("beatels  ");
  });
});

describe("OnboardingState", () => {
  it("invites the user to add music when the library really is empty", async () => {
    const user = userEvent.setup();
    const onScan = vi.fn();
    render(<OnboardingState scanning={false} onScan={onScan} />);
    expect(screen.getByText("Your music lives here")).toBeInTheDocument();
    expect(
      screen.queryByText(/No results for/),
      "a genuinely empty library is not a failed search",
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Scan a folder" }));
    expect(onScan).toHaveBeenCalledOnce();
  });

  it("changes to a progress note while scanning, and cannot be re-triggered", () => {
    render(<OnboardingState scanning={true} onScan={vi.fn()} />);
    expect(screen.getByText("Building your library…")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Scanning…" }),
      "a scan already running must not be started again",
    ).toBeDisabled();
  });
});