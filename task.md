## 1. Whitelist va Xavfsizlik (Security Layer)
### 1.1 Alias-Based Whitelist

Bitta jadval (table) bir necha xil alias bilan ishlatilishi mumkin. Har bir alias o'zining ruxsat etilgan ustunlariga (fields) ega bo'lishi kerak.

- **Sintaksis:** `"table_name:alias_name": ["field1", "field2"]`
- **Qoida:** Agar `@source` ichida alias ishlatilsa, faqat o'sha alias uchun belgilangan whitelist tekshiriladi.

### 1.2 Custom Field Mapping (Alias fields)
Jadval ustunlariga ixtiyoriy nom berish va ularni faqat shu nom bilan chaqirish:

- **JSON format:** `{"alias_name": "real_column_name"}`
- **Misol:** `"a_table:third": {"unique_number": "id"}` bo'lsa, so'rovda `id` emas, `unique_number` ishlatilishi shart.


## 2. Advanced SQL Features
### 2.1 CTE (Common Table Expressions) - Soxta Jadvallar
Backend murakkab virtual jadvallarni (CTE) yuborishi mumkin. Bu parametrlar sonini kamaytirish va loyiha arxitekturasini tozalashga yordam beradi.

- **Yechim:** `CTE` larni `whitelist`ning bir qismi yoki alohida `const char* cte_input` sifatida qabul qilish.
- **Ishlatilishi:** Yasalgan CTE nomi `whitelist` ichida xuddi oddiy jadvaldek ruxsat etilgan bo'lishi kerak.

CTE'larni qanday yasashni maslahat berasan?
Mendagi taklif: `cte`niham json_input kabi formatda yuborish rust uni cte table sifatida yasab oladi!? 


## 3. Fayl tizimi bilan ishlash (Binary/Base64)
### 3.1 `root_files_path` Parametri
   `uaq_parse` funksiyasining 4-ixtiyoriy parametri:
   `uaq_parse(json_input, whitelist, relations, root_files_path)`

### 3.2 File-to-Base64 Conversion
Agar ustun qiymati fayl manzili bo'lsa (masalan: `/2026/04/photo.jpg`), kutubxona:

- `root_files_path` + `column_value`ni birlashtiradi.
- Faylni diskdan o'qiydi.
- Uni **Base64** formatiga o'girib, natija (output JSON) ichiga joylashtiradi.