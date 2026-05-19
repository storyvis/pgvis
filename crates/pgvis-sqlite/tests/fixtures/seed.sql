-- Seed data for SQLite integration tests

INSERT INTO users (id, username, email, bio, is_active) VALUES
    (1, 'alice', 'alice@example.com', 'Software engineer', 1),
    (2, 'bob', 'bob@example.com', NULL, 1),
    (3, 'charlie', 'charlie@example.com', 'Designer', 0);

INSERT INTO items (id, name, price, description, metadata, user_id, is_published) VALUES
    (1, 'Widget', 9.99, 'A useful widget', '{"color":"blue","weight":0.5}', 1, 1),
    (2, 'Gadget', 19.99, 'An amazing gadget', '{"color":"red","weight":1.0}', 1, 1),
    (3, 'Doohickey', 4.99, NULL, NULL, 2, 0),
    (4, 'Thingamajig', 29.99, 'Premium thing', '{"color":"gold"}', NULL, 1);

INSERT INTO tags (id, name) VALUES
    (1, 'electronics'),
    (2, 'sale'),
    (3, 'new');

INSERT INTO item_tags (item_id, tag_id) VALUES
    (1, 1),
    (1, 2),
    (2, 1),
    (2, 3),
    (4, 2);

INSERT INTO orders (id, user_id, total, status, notes) VALUES
    (1, 1, 29.98, 'completed', 'First order'),
    (2, 1, 4.99, 'pending', NULL),
    (3, 2, 19.99, 'shipped', 'Urgent');

INSERT INTO order_items (order_id, item_id, quantity) VALUES
    (1, 1, 2),
    (1, 2, 1),
    (2, 3, 1),
    (3, 2, 1);

INSERT INTO categories (id, name, parent_id) VALUES
    (1, 'Electronics', NULL),
    (2, 'Computers', 1),
    (3, 'Laptops', 2),
    (4, 'Clothing', NULL);

INSERT INTO logs (message, level) VALUES
    ('User logged in', 'info'),
    ('Payment processed', 'info'),
    ('Error occurred', 'error');

INSERT INTO user_settings (user_id, key, value) VALUES
    (1, 'theme', 'dark'),
    (1, 'language', 'en'),
    (2, 'theme', 'light');

INSERT INTO products (id, name, unit_price, quantity) VALUES
    (1, 'Bolt', 0.10, 1000),
    (2, 'Nut', 0.08, 2000);

INSERT INTO unicode_data (id, label, description) VALUES
    (1, 'Ünïcödé', 'Testing unicode: こんにちは 🌍'),
    (2, 'Ñoño', 'Spanish characters: ñ, ¿, ¡');

INSERT INTO nullable_test (id, required_col, optional_col, optional_int, optional_bool) VALUES
    (1, 'has_all', 'present', 42, 1),
    (2, 'only_required', NULL, NULL, NULL);
