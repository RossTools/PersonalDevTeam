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
