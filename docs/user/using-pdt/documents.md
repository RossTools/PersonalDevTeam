# Documents

**Documents** keeps design docs, plans and working spreadsheets outside your repository, linked to the projects and
tickets they're about, so you and your agents can find them from either side.

A document is a **doc** (text, read as Markdown) or a **sheet** (a spreadsheet). A **provider** keeps its content:
PDT's local documents, in a folder on this machine, or a plugin's, such as Google Docs and Sheets. PDT keeps the
document's record and links. Until a provider is turned on, there is nothing to make or import.

## Browse and find

Open **Documents** in the rail. The list shows every document, most recently changed first. Narrow it by project,
**Docs** or **Sheets**, provider or ticket (**Ticket…**), and search by title.

Pick a document to see its links and content. A sheet shows as a table; hover a cell for its formula. **Open in …**
opens it in the provider's own app, when it has one.

## Make or import a document

- **New document** (the pencil at the top of the list): give it a title, pick **Doc** or **Sheet**, the provider and
  the project.
- **Import a document** (beside it): search a provider's files and pick one. PDT links the file; it doesn't copy it.

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

## Rename or remove

**Rename** (the pencil beside the title, or the **⋯** menu) renames it, at the provider too when it can. In the **⋯**
menu, **Remove from PDT…** forgets the document and its links but leaves the file; **Delete document…** deletes the
file too, where the provider allows it.

## Where new documents go

**Settings › This host › Documents** sets the **Default provider** and the **Local documents folder** (blank: PDT's
data folder). **Settings › the project › Documents** sets the same for one project; blank follows this host.

## Google Docs and Sheets

With the **Google** plugin on, your Google Docs and Sheets can be documents. PDT asks Google only for the files it
makes and the ones you pick: it never sees the rest of your Drive.

### Set up a Google Cloud OAuth client

PDT signs in with an OAuth client of your own, made once in the Google Cloud console:

1. Make a project (or pick one) and turn on the **Google Drive API**, **Google Docs API**, **Google Sheets API** and
   **Google Picker API** for it.
2. Set up its **OAuth consent screen** (Google Auth Platform). While it is in testing, add your Google accounts as
   test users.
3. Under **Clients**, make an OAuth client of type **Web application**, and add the address PDT shows under **Sign
   in** on the Google plugin's page to its **Authorized redirect URIs**. That address follows where you opened PDT:
   Google takes plain `http` only for this machine's own address, so sign in from here, or from PDT's HTTPS address
   ([remote access](../your-machine/remote-access.md)) with that address added too.
4. For Google's file picker, make an **API key** (**Credentials › Create credentials › API key**), limited to the
   **Google Picker API**, and note the project's **number** from its dashboard.

### Add a Google account

1. Open **Settings › Plugins › Google**, turn it on, and **Add** a connection: a name, the OAuth client's ID and
   secret, the picker's API key and project number, and, if you like, the **Folder for new documents** (a Drive
   folder's address; blank: My Drive).
2. Under **Sign in**, click **Sign in with Google**. Google's page opens in a window of its own; allow access and it
   closes. The account shows **Signed in as** its email.

Add one connection for each Google account. **Sign out** takes PDT's access back.

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

