-- Converts result_payload to TEXT to align with the domain string payload.
ALTER TABLE processed_messages
    ALTER COLUMN result_payload TYPE TEXT USING result_payload::text;
