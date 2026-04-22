use super::Guard;

impl Guard {
    pub fn check_global_threats(input: &str) -> Result<(), String> {
        let upper = input.to_uppercase();
        
        // Block comment and logic terminators
        if input.contains("--") || input.contains("/*") || input.contains("*/") || input.contains(";") {
            return Err(format!("Comment sequences or terminators are strictly forbidden. Input: {}", input));
        }

        // Block structural manipulation and multi-queries.
        // FIX #5: SELECT added — no legitimate user field/table input needs this keyword.
        //         Internal SQL generation never passes through check_global_threats.
        let bad_words = [
            "DROP ", "DELETE ", "UPDATE ", "INSERT ", "EXEC ", "TRUNCATE ",
            "ALTER ", "GRANT ", "REVOKE ", "UNION ", "UNION(",
            "INTO ", "LOAD_FILE", "OUTFILE", "DUMPFILE",
            "PG_SLEEP", "PG_READ", "PG_WRITE", "PG_STAT",
            "INFORMATION_SCHEMA", "PG_CATALOG",
            "COPY ", "EXECUTE ", "PERFORM ",
            // Subquery / data-exfiltration via SELECT in any user-supplied string:
            "SELECT ", "SELECT(",
        ];
        
        for word in bad_words {
            if upper.contains(word) {
                return Err(format!("Forbidden SQL operation detected: {}", word.trim()));
            }
        }
        
        Ok(())
    }
}
