-- Waste used to be one movement kind for spoiled, broken and stolen stock,
-- which made the stock history reason filter unable to answer "how much did
-- we lose to spoilage?". The cause is an optional detail on a waste movement:
-- old rows keep a NULL cause and still read as "Waste".
ALTER TABLE stock_movements ADD COLUMN cause TEXT
    CHECK (cause IS NULL OR cause IN ('damage', 'spoilage', 'loss'));
