# Remote access and your phone

Reach PDT from your phone or another computer over [Tailscale](https://tailscale.com), with only you allowed in.

PDT normally answers only on the machine it runs on. Remote access serves it over HTTPS on your tailnet (your private
Tailscale network). Nothing is opened to the internet, and only the Tailscale user you name as its owner gets in.

## Set it up

1. **Install Tailscale** on the machine running PDT and on your phone or other computer, and sign in to the same
   tailnet on each.
2. **Turn on HTTPS for your tailnet**: in the Tailscale admin console, open **DNS**, make sure **MagicDNS** is on, and
   click **Enable HTTPS**.
3. In PDT, open **Settings › This host › Remote access** and turn on **Serve on the tailnet**.
4. Under **Owner**, enter your Tailscale login (the email you sign in to Tailscale with), or click **Use <login>** if
   PDT offers it.
5. **Status** shows **Active** and your PDT address, like `https://my-mac.your-tailnet.ts.net`. Open it on your phone.

Set the owner on the machine itself: until there is one, PDT turns every remote request away.

## Who can reach it

- **On the machine itself**: anyone using it, at `http://127.0.0.1:7420`.
- **Over the tailnet**: only the owner. Requests from other tailnet users, shared or tagged devices, or Tailscale
  Funnel are refused.
- **From the internet**: nobody. PDT doesn't use Funnel and opens no port.

Runs started from PDT (a dev server, say) get an HTTPS address on the tailnet too, so their links work from your
phone. Only the owner gets through to them, as with PDT itself: PDT checks each request before passing it on to
the run, WebSockets included, and the run sees the request as Tailscale sent it. A run's address goes away when the run ends, when it comes back on another port after PDT restarts, and when
you turn remote access off, including addresses PDT made before it last restarted. See
[Run configuration](../automation/run-configuration.md).

To open PDT on its own machine by another name (an alias in your hosts file), add it under **Allowed hosts**: see
[Settings](settings.md#network).

## When the status isn't Active

PDT keeps trying in the background, so it picks Tailscale up when Tailscale starts after it. **Check again** tries
now. If it stays **Not active**, the status says why. The usual reasons:

- **Tailscale isn't running, or isn't signed in.** Open Tailscale on that machine and connect.
- **HTTPS certificates are not enabled for this tailnet.** Do step 2 above.
- **Tailscale isn't installed where PDT looks.** **Settings › This host › Checks** shows whether PDT found it.

If the page opens but says you're not the owner, check that the login under **Owner** is the one your phone signs in
to Tailscale with.

## Using PDT on a phone

Open your PDT address in the phone's browser; add it to your home screen to reach it quicker.

- **The top bar** has **☰** (the menu), the screen's title, the bell (what **Needs you**, with its count), search, and
  the screen's own actions.
- **The menu** switches views (Project manager, Sessions, Inbox, Tickets and the rest), lists your workspaces and
  their sessions, and has the theme switch and **Settings**. At the bottom it shows whether your phone can reach PDT.
- **The Inbox** is the place to start: **Review all** steps through what waits for you one item at a time. See
  [The Inbox](../using-pdt/the-inbox.md#from-your-phone).
- **Terminals** get a key bar above the keyboard with the keys a phone lacks (**Esc**, **Tab**, **Ctrl**, the arrows
  and more). See [Workspaces and sessions](../using-pdt/workspaces-and-sessions.md#the-terminals-keys).
- **Tickets, the board and reviews** have phone layouts: see [Tickets and the board](../using-pdt/tickets-and-the-board.md#on-a-phone)
  and [Review and merge](../using-pdt/review-and-merge.md#on-a-phone).
- **Notifications**: on the phone, turn on **Desktop notifications** under **This device** in **Settings ›
  Notifications**. See [Notifications](../using-pdt/the-inbox.md#notifications).
- **Settings** show one screen per section.
