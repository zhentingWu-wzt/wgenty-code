import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { AppTopbar } from "./AppTopbar";
import { useUiStore } from "../../state/uiStore";

describe("AppTopbar", () => {
  beforeEach(() => {
    useUiStore.setState({ theme: "system", leftCollapsed: false, rightPanel: null });
  });

  it("renders brand and toggles left sidebar via uiStore", async () => {
    render(<AppTopbar />);
    expect(screen.getByText("wgenty-code")).toBeInTheDocument();
    await userEvent.click(screen.getByTitle("Toggle sidebar"));
    expect(useUiStore.getState().leftCollapsed).toBe(true);
  });

  it("switches theme via dropdown", async () => {
    render(<AppTopbar />);
    await userEvent.click(screen.getByTitle("Theme"));
    await userEvent.click(await screen.findByText("Dark"));
    expect(useUiStore.getState().theme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });

  it("toggles right rail", async () => {
    render(<AppTopbar />);
    await userEvent.click(screen.getByTitle("Toggle right panel"));
    expect(useUiStore.getState().rightPanel).toBe("sessions");
    await userEvent.click(screen.getByTitle("Toggle right panel"));
    expect(useUiStore.getState().rightPanel).toBeNull();
  });
});
