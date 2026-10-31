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

## Work on documents with agents and the PM

Agents and the PM read and change documents themselves, whichever provider keeps them. Ask the PM for a budget sheet
or an agent for a design and it writes a document linked to the ticket or project, never a file in your repository. In
a doc they replace or add sections, change text and fill in tables; in a sheet they write values and formulas, add,
rename and delete sheets, format cells, add conditional formats and charts, and sort and filter, as far as the provider
can. They read a document's comments, reply to them and resolve them.

Reading never asks. Changing a document, making one, linking one, and replying to or resolving a comment ask you
first, as the PM's other changes do, unless the project lets the PM act without asking (**Settings › the project › Pipeline and policies**). What other
people wrote in a document's comments reaches agents marked as outside input, never as instructions.

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

**Rename** (the pencil beside the title, or the **⋯** menu) renames the document, at the provider too when it can.
In the **⋯** menu, **Remove from PDT…** forgets the document and its links but leaves the file where it is;
**Delete document…** deletes the file too, where the provider allows it.

## Where new documents go

- **Settings › This host › Documents**: the **Default provider** for new documents, and the **Local documents folder**
  where documents kept on this machine are saved (blank: PDT's data folder).
- **Settings › the project › Documents**: the same two for one project; blank follows this host.
