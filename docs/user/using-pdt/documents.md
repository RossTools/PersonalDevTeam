# Documents

**Documents** keeps design docs, plans and working spreadsheets outside your repository, linked to the projects and
tickets they're about, so you and your agents can find them from either side.

A document is a **doc** (text, read as Markdown) or a **sheet** (a spreadsheet). A **provider** keeps its content:
**Local**, PDT's own, which saves each document as a file on this machine and needs no account, or a plugin's, such
as Google Docs and Sheets. PDT keeps the document's record and links.

## Browse and find

Open **Documents** in the rail. The list shows every document, most recently changed first. Narrow it by project,
**Docs** or **Sheets**, provider or ticket (**Ticket…**), and search by title.

Pick a document to see its links, content and comments. A sheet from a plugin's provider shows as a table; hover a
cell for its formula. **Open in …** opens it in the provider's own app, when it has one.

## Write a doc

A local doc opens **Rendered**. Switch to **Write** to edit its Markdown; it's saved as you type ("Saved" shows when
it is). **Add image** puts a picture in the doc, saved beside the doc's file.

If an agent (or you, in another window) saves the doc while you have unsaved changes, nothing is overwritten: a
notice asks you to **Keep mine** (theirs is replaced) or **Take theirs** (your changes go). While you have nothing
unsaved, changes made elsewhere show at once.

## Work in a sheet

A local sheet opens in a spreadsheet editor: formulas, number formats, fonts, fills, borders, merged cells, frozen
rows and several sheets (the tabs at the bottom). Every change is saved as you make it. Above the grid:

- **Sort A→Z by …** and **Z→A** sort by the selected cell's column: the selected range, or with none every row below
  the first (the header row).
- **Only rows with “…” in …** hides rows whose cell in that column shows something else; **Show all rows** brings
  them back.
- **Chart the selection** charts the selected cells, headers included: the first column labels it, each other column
  is a series named by its header. Pick **Column**, **Bar**, **Line**, **Area**, **Pie** or **Scatter** and a title.
  The chart floats beside the cells and redraws when they change; its **⋯** menu changes its kind or removes it.

An agent's change to an open sheet shows at once, its formulas recalculated. If you and an agent change it at the
same moment, the last change wins and a notice says the sheet now shows both.

## Comment

Local documents have **Comments** under them. **Comment** starts one: on a doc, about the whole document, a heading,
or the text you selected before clicking; on a sheet, about the selected cells (click the cell name above a comment
to select them again). Reply, **Resolve** when it's done (it folds to one line), or **Reopen**. Agents read, add,
answer and resolve comments too.

## Make, import or export a document

- **New document** (the pencil at the top of the list): give it a title, pick **Doc** or **Sheet**, the provider and
  the project.
- **Import a document** (beside it): search a provider's files and pick one. PDT links the file; it doesn't copy it.
  **Local** lists the `.md` and `.xlsx` files in the local documents folders.
- **From your computer…** (in **Import a document**): a `.md` file becomes a doc, a `.xlsx` or `.csv` file a sheet,
  copied into the local documents folder. A spreadsheet's charts, pictures and pivot tables aren't kept; PDT says so.
- The **⋯** menu exports a doc as **Export as Markdown (.md)**, and a sheet as **Export as Excel (.xlsx)** (every
  sheet, with formulas and formats) or as CSV (the sheet you're on). Charts aren't in the exported file.

## Link documents to projects and tickets

On a document's page, **Link a project** and **Link a ticket** add links; the **×** on a link removes it. A document
can belong to any number of projects and tickets.

A ticket's **Documents** section lists its documents: **Add** links an existing one, makes a new one or imports one,
and the unlink button takes one off the ticket (the document stays). A project's documents are under **Settings › the
project › Documents**.

## Create a design doc

**Create design doc** in a ticket's **Documents** section makes a doc named after the ticket from a short template
(**Goal**, **Scope**, **Out of scope**, **Done when**, **Notes**), in the project's default provider, links it to the
ticket and project, and opens it. Agents working on the ticket see its documents and are told to read the design doc
before they start.

## Work on documents with agents and the PM

Agents and the PM read and change documents themselves, whichever provider keeps them. Ask the PM for a budget sheet
or an agent for a design and it writes a document linked to the ticket or project, never a file in your repository. In
a doc they replace or add sections, change text and fill in tables; in a sheet they write values and formulas, add,
rename and delete sheets, format cells, add conditional formats and charts, and sort and filter, as far as the provider
can. They read a document's comments, reply to them and resolve them.

Reading never asks. Changing a document, making one, linking one, and replying to or resolving a comment ask you
first, as the PM's other changes do, unless the project lets the PM act without asking (**Settings › the project › Pipeline and policies**). What other
people wrote in a document's comments reaches agents marked as outside input, never as instructions.

Workflows work on documents too: they can make one, add text or rows, set cells, add a chart and comment, and start
when a document changes or gets a new comment. See [Workflows](../automation/workflows.md#steps), with examples that
log merged tickets to a sheet and have an agent answer comments.

## Documents beside a conversation

A document can sit in a panel beside an agent's conversation or a PM conversation, so you watch the work land as it
happens: what an edit changed lights up for a moment.

- **Open a document beside…** in a conversation's **⋯** menu (in a PM conversation, **Open beside…** at the top)
  picks one of your documents.
- **Open beside** on a document's **⋯** menu, or the panel button on a row of a ticket's **Documents**, opens it beside
  one of the ticket's conversations or a PM conversation, and takes you there.
- An agent or the PM can open one there itself ("I've drafted the plan; it's open on the right").

Several documents open as tabs. Click a tab to show it, drag it to move it (or use its right-click menu, a long press
on touch screens), and **×** closes it. The panel's buttons open another document, open the one shown in
**Documents**, and collapse the panel to a strip at the edge; drag the panel's left edge to make it wider. The tabs stay
with the conversation, and the panel's width and whether it is collapsed are remembered per conversation.

On a phone, a **Conversation | Documents** switch at the top of the conversation flips between the two.

## Rename or remove

**Rename** (the pencil beside the title, or the **⋯** menu) renames it, at the provider too when it can (a local
document keeps its file name). In the **⋯** menu, **Remove from PDT…** forgets the document and its links but leaves
the file; **Delete document…** deletes the file too, where the provider allows it (a local doc's pictures go with it).

## Where new documents go

**Settings › This host › Documents** sets the **Default provider** and the **Local documents folder** (blank: PDT's
data folder). **Settings › the project › Documents** sets the same for one project; blank follows this host.

A local doc is a `.md` file (its pictures in a `.assets` folder beside it), a local sheet a `.xlsx` you can open in
Excel, LibreOffice or Google Sheets; comments and charts are in PDT's database. Changes made to a file outside PDT
show within a few seconds while it's open. The default folder is inside PDT's data folder, so backing that up
includes your documents ([Your data](../your-machine/your-data.md#backups)); back up a folder elsewhere too.

## Google Docs and Sheets

With the **Google** plugin on, your Google Docs and Sheets can be documents. PDT asks Google only for the files it
makes and the ones you pick: it never sees the rest of your Drive.

### Add a Google account

1. Open **Settings › Plugins › Google** and turn it on.
2. Under **Sign in**, click **Sign in with Google**. Google's page opens in a window of its own; pick your account and
   allow access. Google sends you back through a page on pdt.dev that asks **Return to PDT at …?**: check it's the
   address you opened PDT at and click **Return to PDT**. The window closes and the account shows, named after its
   email.

This works wherever you opened PDT: on this machine, from another device on your network, or over
[Tailscale](../your-machine/remote-access.md). Sign in once for each Google account. **Sign out** takes PDT's access
back; you can also remove it in your Google Account (**Security › Third-party apps with account access**).

To choose where new documents go, **Edit** the account's connection and set **Folder for new documents** (a Drive
folder's address; blank: My Drive).

### Let agents use an account

Agents use a Google account only once you allow them: click **Allow agents** beside it and agree on Google's page (a
second consent, for PDT's agents). Until then, agents' document tools say they may not use that account's Docs and
Sheets, and never act with your own sign-in. **Stop agents** takes the permission away again; your own sign-in stays.

### Use your own OAuth client

If your Google Workspace organization blocks apps from outside it, sign in with an OAuth client of your own instead,
made once in the Google Cloud console:

1. Make a project (or pick one) and turn on the **Google Drive API**, **Google Docs API**, **Google Sheets API** and
   **Google Picker API** for it.
2. Set up its **OAuth consent screen** (Google Auth Platform). While it is in testing, add your Google accounts as
   test users.
3. Under **Clients**, make an OAuth client of type **Web application**, and add the address PDT shows under
   **Advanced** on the Google plugin's page to its **Authorized redirect URIs**. That address follows where you
   opened PDT: Google takes plain `http` only for this machine's own address, so sign in from here, or from PDT's HTTPS
   address ([remote access](../your-machine/remote-access.md)) with that address added too.
4. For Google's file picker, make an **API key** (**Credentials › Create credentials › API key**), limited to the
   **Google Picker API**, and note the project's **number** from its dashboard.

Then, under **Advanced**, click **Use your own OAuth client** and fill in its ID and secret, the picker's API key and
project number. Save, and click **Sign in** beside the account. Agents use such an account through its sign-in, with
no separate permission.

A PDT built without its own Google sign-in (one you built yourself) offers only this way: **Add** a connection with
your client's details, then **Sign in**.

### Pick files from your Drive

In **Import a document**, choose **Google** and click **Pick from Google Drive**: Google's own picker searches your
whole Drive. The Docs and Sheets you pick become documents, and PDT can open them from then on. The list under the
button shows the files PDT already has access to.

New Google documents are made in the first signed-in account (by connection name), in its folder.

### Work on them live

A Google document's page shows Google's own editor, so you see agents' edits as they land and can edit alongside
them. **Read in PDT** shows PDT's own reading of it instead, which updates when the document changes. PDT remembers
which you chose.

Some browsers (Safari, and PDT's desktop app on a Mac) keep Google's sign-in out of pages shown inside PDT, so the
editor asks you to sign in or stays blank. Use **Read in PDT**, or **Open in Google**.

Agents can change a Doc's text (whole sections, after a heading, find and replace, with headings, lists, tables and
links) and a Sheet fully: values and formulas, sheets, formatting, borders, column widths, frozen rows, conditional
formatting, validation, charts, sorting and filters. They read and answer comments too, and resolve them.

When Google limits how often PDT may call it, PDT says so: try again a minute later. If an account's sign-in expired
or was taken back, PDT says to sign in again.

