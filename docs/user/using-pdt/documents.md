# Documents

Design docs, plans and working spreadsheets don't have to live in your repository. **Documents** keeps them outside
it, linked to the projects and tickets they're about, so you and your agents can find them from either side.

A document is a **doc** (text, read as Markdown) or a **sheet** (a spreadsheet). Each one is kept by a **provider**:
PDT's own local documents, saved in a folder on this machine, or a plugin's, such as Google Docs and Sheets. PDT keeps
a record of each document and what it's linked to; the content stays with the provider. Until a provider is turned
on, the Documents view has nothing to make or import.

## Browse and find

Open **Documents** in the rail. The list shows every document, the most recently changed first, with its projects,
tickets and provider. Narrow it by project, by **Docs** or **Sheets**, by provider, or by ticket (**Ticket…**), and
search by title.

Pick a document to see it: its title, who added it and when, its links, and its content as the provider reads it. A
sheet shows as a table; hover a cell to see its formula. **Open in …** opens it in the provider's own app, when it has
one.

## Make or import a document

- **New document** (the pencil at the top of the list): give it a title, pick **Doc** or **Sheet**, the provider and
  the project it belongs to. It opens once it's made.
- **Import a document** (beside it): search a provider's files and pick one. PDT links the file you already have; it
  doesn't copy it. Importing a file PDT already knows just links it again.

## Link documents to projects and tickets

On a document's page, **Link a project** and **Link a ticket** add links; the **×** on a link removes it. A document
can belong to any number of projects and tickets.

Each ticket has a **Documents** section (on a phone, a fold of the ticket screen) that lists its documents. **Add**
links one PDT already has, makes a new one, or imports one. The unlink button takes a document off the ticket; the
document itself stays.

A project's documents are on its settings page, under **Settings › the project › Documents**.

## Create a design doc

**Create design doc** in a ticket's **Documents** section makes a doc named after the ticket (its key, then "design:" and its title) from
a short template (**Goal**, **Scope**, **Out of scope**, **Done when**, **Notes**), in the project's default provider,
links it to the ticket and its project, and opens it.

Agents working on the ticket see its documents when they read the ticket, and are told to read the design doc before
they start.

## Rename or remove

**Rename** (the pencil beside the title, or the **⋯** menu) renames the document, at the provider too when it can.
In the **⋯** menu, **Remove from PDT…** forgets the document and its links but leaves the file where it is;
**Delete document…** deletes the file too, where the provider allows it.

## Where new documents go

- **Settings › This host › Documents**: the **Default provider** for new documents, and the **Local documents folder**
  where documents kept on this machine are saved (blank: PDT's data folder).
- **Settings › the project › Documents**: the same two for one project; blank follows this host.
