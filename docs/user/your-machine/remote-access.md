# Remote access and your phone

Reach PDT from your phone or another computer over [Tailscale](https://tailscale.com), with only you allowed in.

Out of the box PDT only answers on the machine it runs on. Remote access puts it on your tailnet (your private
Tailscale network) over HTTPS. Nothing is opened to the internet, and PDT lets in only the Tailscale user you name as
its owner.

## Set it up

1. **Install Tailscale** on the machine running PDT and on your phone or other computer, and sign in to the same
   tailnet on each.
2. **Turn on HTTPS for your tailnet**: in the Tailscale admin console, open **DNS**, make sure **MagicDNS** is on, and
   click **Enable HTTPS**. PDT needs it to serve over HTTPS.
3. In PDT, open **Settings › This host › Remote access** and turn on **Serve on the tailnet**.
4. Under **Owner**, enter your Tailscale login (the email you sign in to Tailscale with), or click **Use <login>** if
   PDT offers it. Only this user gets in.
5. **Status** shows **Active** and your PDT address, like `https://my-mac.your-tailnet.ts.net`. Open it on your phone.

Set the owner on the machine itself: until there is one, PDT turns every remote request away.

## Who can reach it

- **On the machine itself**: anyone using it, at `http://127.0.0.1:7420`.
- **Over the tailnet**: only the owner. A request from another tailnet user, a shared or tagged device, or Tailscale
  Funnel is refused.
- **From the internet**: nobody. PDT doesn't use Funnel and doesn't open any port.

Runs started from PDT (a dev server, say) get an HTTPS address on the tailnet too, so their links work from your
phone. See [Run configuration](../automation/run-configuration.md#pitfalls).

## When the status isn't Active

PDT keeps trying in the background: Tailscale often starts after PDT at login, and PDT picks it up when it does.
**Check again** tries now. If it stays **Not active**, the status says why. The usual reasons:

- **Tailscale isn't running, or isn't signed in.** Open Tailscale on that machine and connect.
- **HTTPS certificates are not enabled for this tailnet.** Do step 2 above.
- **Tailscale isn't installed where PDT looks.** **Settings › This host › Checks** shows whether PDT found it.

If the page opens but says you're not the owner, check the login under **Owner** matches the one your phone signs in
to Tailscale with.

## Using PDT on a phone

PDT's page fits a phone screen. Open your PDT address in the phone's browser; adding it to your home screen makes it
quicker to reach.

- **The top bar** has **☰** (the menu), the screen's title, the bell (what **Needs you**, with its count), search, and
  the screen's own actions.
- **The menu** switches views (Project manager, Sessions, Inbox, Tickets and the rest), lists your workspaces and
  their sessions, and has the theme switch and **Settings**. At the bottom it shows whether your phone can reach PDT.
- **The Inbox** is the place to start: **Review all** steps through what waits for you one item at a time. See
  [The Inbox](../using-pdt/the-inbox.md#from-your-phone).
- **Terminals** get a key bar above the keyboard with the keys a phone lacks (**Esc**, **Tab**, **Ctrl**, the arrows
  and more). See [Workspaces and sessions](../using-pdt/workspaces-and-sessions.md#the-terminals-keys).
- **Tickets, the board and reviews** have phone layouts too: see [Tickets and the board](../using-pdt/tickets-and-the-board.md#on-a-phone)
  and [Review and merge](../using-pdt/review-and-merge.md#on-a-phone).
- **Notifications**: turn on **Desktop notifications** under **This device** in **Settings › Notifications** on the
  phone to get notified there. See [Notifications](../using-pdt/the-inbox.md#notifications).

Settings work on a phone too, one screen per section.

## Other host names

PDT listens only on the machine it runs on: the network can't reach it except through Tailscale. If you open it on
that machine by another name (an alias in your hosts file, say), add the name under **Settings › This host › Network ›
Allowed hosts** and restart PDT; otherwise PDT refuses the page, as a guard against websites that try to reach it
through your browser. See [Settings](settings.md#network).
