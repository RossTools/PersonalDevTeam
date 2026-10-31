# Documents

Design docs, plans and working spreadsheets don't have to live in your repository. **Documents** keeps them outside
it, linked to the projects and tickets they're about, so you and your agents can find them from either side.

A document is a **doc** (text, read as Markdown) or a **sheet** (a spreadsheet). Each one is kept by a **provider**:
**Local**, PDT's own, which saves each document as a file in a folder on this machine and needs no account, or a
plugin's, such as Google Docs and Sheets. PDT keeps a record of each document and what it's linked to; the content
stays with the provider.

## Browse and find

Open **Documents** in the rail. The list shows every document, the most recently changed first, with its projects,
tickets and provider. Narrow it by project, by **Docs** or **Sheets**, by provider, or by ticket (**Ticket…**), and
search by title.

Pick a document to see it: its title, who added it and when, its links, its content and its comments. A sheet from a
plugin's provider shows as a table; hover a cell to see its formula. **Open in …** opens it in the provider's own app,
when it has one.

## Write a doc

A local doc opens **Rendered**. Switch to **Write** to edit its Markdown: it's saved as you type ("Saved" shows when it
is). **Add image** puts a picture in the doc; it's saved beside the doc's file.

If an agent (or you, in another window) saves the doc while you have changes not saved yet, nothing is overwritten:
a notice asks you to **Keep mine** (theirs is replaced) or **Take theirs** (your changes go). While you have nothing
unsaved, changes made elsewhere show at once.

## Work in a sheet

A local sheet opens in a full spreadsheet editor: formulas, number formats, fonts, fills, borders, merged cells,
frozen rows and several sheets (the tabs at the bottom). Every change is saved as you make it.

Above the grid:

- **Sort A→Z by …** and **Z→A** sort by the column of the selected cell. With a range selected, only that range is
  sorted; otherwise every row below the first (the header row) is.
- **Only rows with “…” in …** hides the rows whose cell in that column shows something else; **Show all rows** brings
  them back.
- **Chart the selection** makes a chart of the selected cells, headers included: the first column labels it, each
  other column is a series named by its header. Pick **Bar**, **Line**, **Area**, **Pie** or **Scatter** and a title.
  The chart floats beside the cells and redraws whenever they change; its **⋯** menu changes its kind or removes it.

When an agent changes the sheet while you have it open, its change shows at once and its formulas recalculate. If
you and an agent change it at the same moment, the last change wins and a notice says the sheet now shows both.

## Comment

Every local document has **Comments** under it. **Comment** starts one:

- On a doc, pick what it's about: the whole document, a heading, or the text you selected on the page before you
  clicked **Comment**.
- On a sheet, it goes on the cells you have selected. Click the cell name above a comment to select those cells
  again.

Reply to a comment, or **Resolve** it when it's done; resolved comments fold to one line, and **Reopen** opens one
again. Agents read, add, answer and resolve comments too.

## Export

The **⋯** menu exports a doc as Markdown (**Export as Markdown (.md)**), and a sheet as **Export as Excel (.xlsx)**
(every sheet, with its formulas and formats) or as CSV (the sheet you're on). Charts are not in the exported file.

## Make or import a document

- **New document** (the pencil at the top of the list): give it a title, pick **Doc** or **Sheet**, the provider and
  the project it belongs to. It opens once it's made.
- **Import a document** (beside it): search a provider's files and pick one. PDT links the file you already have; it
  doesn't copy it. Importing a file PDT already knows just links it again. **Local** lists the `.md` and `.xlsx` files
  in the local documents folders.
- **From your computer…** (at the bottom of **Import a document**): pick a `.md` file to make a doc, or a `.xlsx` or
  `.csv` file to make a sheet. PDT copies it into the local documents folder. A spreadsheet's charts, pictures and
  pivot tables aren't kept; PDT says so when the file had any.

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

**Rename** (the pencil beside the title, or the **⋯** menu) renames the document, at the provider too when it can. A
local document keeps its file name.
In the **⋯** menu, **Remove from PDT…** forgets the document and its links but leaves the file where it is;
**Delete document…** deletes the file too, where the provider allows it (a local doc's pictures go with it).

## Where new documents go

- **Settings › This host › Documents**: the **Default provider** for new documents, and the **Local documents folder**
  where documents kept on this machine are saved (blank: PDT's data folder).
- **Settings › the project › Documents**: the same two for one project; blank follows this host.

Each local document is a file in that folder: a doc is a `.md` file (its pictures in a `.assets` folder beside it),
a sheet a `.xlsx` file you can open in Excel, LibreOffice or Google Sheets. Its comments and charts are kept in PDT's
database. Changes you make to a file outside PDT show in PDT the next time it looks, within a few seconds while the
document is open.

The local documents folder is in PDT's data folder unless you pick another, so your usual backup of the data folder
includes your documents (see [Your data](../your-machine/your-data.md#backups)). If you pick a folder elsewhere,
include it in your backups too.
