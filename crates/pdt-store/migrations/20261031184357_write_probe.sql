-- One row PDT writes while writes fail, to see when they work again (PDT-323).
CREATE TABLE write_probe (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    at INTEGER NOT NULL
);
