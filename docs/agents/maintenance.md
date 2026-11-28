# Papercuts and Maintenance Suggestions

- Fixed-tick economy fixtures use `Tuning::default()`, whose material transfer interval is 75 ticks. The tracked `config/balance.ron` uses 24 ticks. Add a shared test helper for timing measured in material pulses so economy playtests expose the chosen interval and remain readable when tuning changes.
