import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import Settings from "./Settings";

/**
 * Issue #33: "the dropdown does nothing, only System default".
 *
 * The list used to be fetched from the click that opens the popup, and the
 * select was disabled while that fetch ran — so the gesture that opened the
 * native popup disabled the control doing the opening, and the list never had a
 * chance to be in the DOM. These tests assert the two facts that failure was
 * made of: the options are there before anyone clicks, and the select is
 * enabled even while a read is still in flight.
 */
const api = vi.hoisted(() => ({
  getAudioDevice: vi.fn(),
  getAudioExclusive: vi.fn(),
  listAudioDevices: vi.fn(),
  setAudioDevice: vi.fn(),
  setAudioExclusive: vi.fn(),
  getCrossfade: vi.fn(),
  setCrossfade: vi.fn(),
  getLogPath: vi.fn(),
  getOnlineCover: vi.fn(),
  setOnlineCover: vi.fn(),
  getTheme: vi.fn(),
  setTheme: vi.fn(),
  listenTheme: vi.fn(),
}));

vi.mock("./api", () => api);

type Device = { name: string; description: string };

const devices: Device[] = [
  { name: "speakers", description: "Speakers (Realtek)" },
  { name: "spdif", description: "S/PDIF out" },
];

beforeEach(() => {
  api.getAudioDevice.mockResolvedValue("");
  api.getAudioExclusive.mockResolvedValue(false);
  api.listAudioDevices.mockResolvedValue(devices);
  api.getCrossfade.mockResolvedValue({ enabled: false, seconds: 3 });
  api.getLogPath.mockResolvedValue(null);
  api.getOnlineCover.mockResolvedValue(false);
  api.getTheme.mockResolvedValue("system");
});

describe("Settings — output device picker (issue #33)", () => {
  const picker = () => screen.findByRole("combobox", { name: "Output device" });

  it("lists the devices mpv returned without needing a click first", async () => {
    render(<Settings />);
    await picker();
    expect(
      await screen.findByRole("option", { name: "Speakers (Realtek)" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "S/PDIF out" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "System default" })).toBeInTheDocument();
  });

  it("is enabled — a select that cannot be opened is the bug", async () => {
    render(<Settings />);
    const select = await picker();
    await waitFor(() => expect(select).toBeEnabled());
    expect(screen.queryByRole("option", { name: "Speakers (Realtek)" })).toBeInTheDocument();
  });

  it("stays enabled while the list is still being read", async () => {
    let answer: (list: Device[]) => void = () => {};
    api.listAudioDevices.mockReturnValue(
      new Promise<Device[]>((resolve) => {
        answer = resolve;
      }),
    );
    render(<Settings />);
    const select = await picker();
    await waitFor(() => expect(api.getAudioDevice).toHaveBeenCalled());
    expect(
      select,
      "disabling the control while it loads is what dismissed the popup",
    ).toBeEnabled();
    answer(devices);
    expect(await screen.findByRole("option", { name: "S/PDIF out" })).toBeInTheDocument();
  });

  it("does not re-read the list merely because the picker was opened", async () => {
    const user = userEvent.setup();
    render(<Settings />);
    await picker();
    await waitFor(() => expect(api.listAudioDevices).toHaveBeenCalledTimes(1));
    await user.click(await picker());
    expect(api.listAudioDevices).toHaveBeenCalledTimes(1);
  });

  it("leaves the picker usable when the stored device cannot be read", async () => {
    api.getAudioDevice.mockRejectedValue(new Error("no database"));
    render(<Settings />);
    const select = await picker();
    await waitFor(() => expect(select).toBeEnabled());
    expect(screen.getByRole("option", { name: "System default" })).toBeInTheDocument();
  });

  it("still saves the device the user picks", async () => {
    const user = userEvent.setup();
    api.setAudioDevice.mockResolvedValue(undefined);
    render(<Settings />);
    await picker();
    await screen.findByRole("option", { name: "S/PDIF out" });
    await user.selectOptions(await picker(), "spdif");
    expect(api.setAudioDevice).toHaveBeenCalledWith("spdif");
  });
});
