CREATE TABLE markdowns (
    url         TEXT NOT NULL UNIQUE PRIMARY KEY,
    parent_url  TEXT NOT NULL,
    hash        TEXT NOT NULL,
    timestamp   INTEGER NOT NULL,
    frontmatter BLOB NOT NULL,
    blocks      TEXT NOT NULL,
    rendered    BLOB NOT NULL
);

CREATE TABLE pages (
    url      TEXT NOT NULL UNIQUE PRIMARY KEY,
    hash     TEXT NOT NULL,
    rendered BLOB NOT NULL
);

CREATE TABLE assets (
    filename TEXT NOT NULL UNIQUE PRIMARY KEY,
    hash     TEXT NOT NULL
);

CREATE TABLE partial_checkpoint (
    id      INTEGER PRIMARY KEY,
    touched INTEGER NOT NULL
);
