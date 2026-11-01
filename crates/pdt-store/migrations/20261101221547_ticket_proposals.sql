-- Ticket proposals (PDT-366): ticket lists leave out tickets with an open proposal item, and the board asks often.
CREATE INDEX inbox_items_ticket_proposal ON inbox_items (ticket_id) WHERE kind = 'ticket_proposal' AND status = 'open';
