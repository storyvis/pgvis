-- SQLite test schema
-- Mirrors the PostgreSQL test schema where applicable

CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    email TEXT,
    bio TEXT,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE items (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    price REAL NOT NULL DEFAULT 0.0,
    description TEXT,
    metadata JSON,
    user_id INTEGER REFERENCES users(id),
    is_published BOOLEAN NOT NULL DEFAULT 0
);

CREATE TABLE orders (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id),
    total REAL NOT NULL DEFAULT 0.0,
    status TEXT NOT NULL DEFAULT 'pending',
    notes TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE order_items (
    order_id INTEGER NOT NULL REFERENCES orders(id),
    item_id INTEGER NOT NULL REFERENCES items(id),
    quantity INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (order_id, item_id)
);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE item_tags (
    item_id INTEGER NOT NULL REFERENCES items(id),
    tag_id INTEGER NOT NULL REFERENCES tags(id),
    PRIMARY KEY (item_id, tag_id)
);

-- Self-referential table
CREATE TABLE categories (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    parent_id INTEGER REFERENCES categories(id)
);

-- Table with no primary key
CREATE TABLE logs (
    message TEXT NOT NULL,
    level TEXT NOT NULL DEFAULT 'info',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

-- Table with composite unique constraint
CREATE TABLE user_settings (
    user_id INTEGER NOT NULL REFERENCES users(id),
    key TEXT NOT NULL,
    value TEXT,
    UNIQUE (user_id, key)
);

-- Simple view
CREATE VIEW active_users AS
    SELECT id, username, email
    FROM users
    WHERE is_active = 1;

-- Table with generated column (SQLite 3.31+)
CREATE TABLE products (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    unit_price REAL NOT NULL,
    quantity INTEGER NOT NULL DEFAULT 0,
    total_value REAL GENERATED ALWAYS AS (unit_price * quantity) STORED
);

-- Unicode table for testing unicode data
CREATE TABLE unicode_data (
    id INTEGER PRIMARY KEY,
    label TEXT NOT NULL,
    description TEXT
);

-- Table with nullable columns for null testing
CREATE TABLE nullable_test (
    id INTEGER PRIMARY KEY,
    required_col TEXT NOT NULL,
    optional_col TEXT,
    optional_int INTEGER,
    optional_bool BOOLEAN
);
