use super::Guard;

impl Guard {
    pub fn check_global_threats(input: &str) -> Result<(), String> {
        let upper = input.to_uppercase();
        
        // Block comment and logic terminators
        if input.contains("--") || input.contains("/*") || input.contains("*/") || input.contains(";") {
            return Err(format!("Comment sequences or terminators are strictly forbidden. Input: {}", input));
        }

        // Block structural manipulation and multi-queries
        let bad_words = [
            "DROP ", "DELETE ", "UPDATE ", "INSERT ", "EXEC ", "TRUNCATE ", 
            "ALTER ", "GRANT ", "REVOKE ", "UNION "
        ];
        
        for word in bad_words {
            if upper.contains(word) {
                return Err(format!("Forbidden SQL operation detected: {}", word));
            }
        }
        
        Ok(())
    }
}
