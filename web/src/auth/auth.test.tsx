import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ApiError } from "../api/client";
import { MOCK_PASSWORD, MockApi } from "../api/mock";
import { App } from "../app/App";
import { AppProvider } from "../app/context";
import { authErrorMessage, loginErrorMessage } from "./LoginPage";

function setup(opts: ConstructorParameters<typeof MockApi>[0]) {
  const api = new MockApi({ liveEvents: false, ...opts });
  render(
    <AppProvider api={api}>
      <App />
    </AppProvider>,
  );
  return { api, user: userEvent.setup() };
}

describe("auth messages", () => {
  it("explains login errors", () => {
    expect(loginErrorMessage(new ApiError(401, "x", "invalid_credentials"))).toMatch(/Wrong/);
    expect(loginErrorMessage(new ApiError(429, "x", "rate_limited", 300))).toMatch(/5 min/);
    expect(loginErrorMessage(new ApiError(404, "x", "disabled"))).toMatch(/turned off/);
    expect(loginErrorMessage(new Error("boom"))).toBe("boom");
  });

  it("explains OIDC errors", () => {
    expect(authErrorMessage(null)).toBeNull();
    expect(authErrorMessage("access_denied")).toMatch(/cancelled/);
    expect(authErrorMessage("weird")).toMatch(/weird/);
  });
});

describe("sign-in flows", () => {
  it("requires login, rejects a wrong password, then signs in", async () => {
    const { user } = setup({ auth: "login" });
    await user.type(await screen.findByLabelText("Username"), "admin");
    await user.type(screen.getByLabelText("Password"), "nope");
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Wrong username or password.");

    await user.clear(screen.getByLabelText("Password"));
    await user.type(screen.getByLabelText("Password"), MOCK_PASSWORD);
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    expect(await screen.findByRole("heading", { name: "Media" })).toBeInTheDocument();
    expect(screen.getByText("admin")).toBeInTheDocument();
  });

  it("offers the OIDC button with a return path", async () => {
    setup({ auth: "login" });
    const link = await screen.findByRole("link", { name: "Sign in with SSO" });
    expect(link.getAttribute("href")).toMatch(/^\/api\/auth\/oidc\/login\?returnTo=/);
  });

  it("creates the first admin on first run", async () => {
    const { user } = setup({ auth: "setup" });
    expect(
      await screen.findByRole("heading", { name: "Create the admin account" }),
    ).toBeInTheDocument();
    const submit = screen.getByRole("button", { name: /Create admin/ });
    await user.type(screen.getByLabelText("Password"), "short");
    expect(submit).toBeDisabled();
    await user.clear(screen.getByLabelText("Password"));
    await user.type(screen.getByLabelText("Password"), "long-enough-pw");
    await user.type(screen.getByLabelText("Confirm password"), "long-enough-pw");
    await user.click(submit);
    expect(await screen.findByRole("button", { name: /Edit/ })).toBeInTheDocument();
  });

  it("signs out back to the login page", async () => {
    const { user } = setup({ auth: "open" });
    await user.click(await screen.findByRole("button", { name: "Sign out" }));
    await waitFor(() =>
      expect(screen.getByRole("heading", { name: "Sign in" })).toBeInTheDocument(),
    );
  });
});

describe("users panel", () => {
  it("adds a user and refuses to remove the last admin", async () => {
    const { user } = setup({ auth: "open" });
    await user.click(await screen.findByRole("button", { name: /Edit/ }));
    await user.click(await screen.findByRole("button", { name: "Users" }));
    await screen.findByText("admin", { selector: "strong" });

    await user.type(screen.getByLabelText("Username"), "guest");
    await user.type(screen.getByLabelText("Password"), "guest-password");
    await user.click(screen.getByRole("button", { name: "Add user" }));
    expect(await screen.findByText("guest", { selector: "strong" })).toBeInTheDocument();

    vi.spyOn(window, "confirm").mockReturnValue(true);
    await user.click(screen.getByRole("button", { name: "Delete admin" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("last admin");
  });
});
