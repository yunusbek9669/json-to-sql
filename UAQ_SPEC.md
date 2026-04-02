🛸 Project: Universal Adaptive JSON-to-SQL Engine (UAQ-Engine)
===========================

## 1. Executive Summary
   Ushbu loyiha frontenddan keladigan deklarativ JSON formatini tahlil qilib, xavfsiz, optimallashgan va parametrli SQL so'rovlarini generatsiya qiluvchi universal kutubxonani yaratishni maqsad qilgan. Kutubxona Rust tilida yoziladi va C-ABI / WebAssembly orqali har qanday dasturlash tilida (PHP, Java, Node.js, Python) "native" tezlikda ishlash imkoniyatiga ega bo'ladi.

## 2. Frontend Request Specification (The Declarative JSON)
   Frontendchi backendga ma'lumotlar strukturasini va filtrlarini bitta ierarxik JSONda yuboradi.

### Key Syntax Elements:
- `@source`: Manba jadval va filtr shartlari.
  - Format: `table_name[field: value, operator value]`
  - Operators: `:` (eq), `!:` (neq), `>` (gt), `<` (lt), `..` (between), `~` (like), `in` (list).

- `@join`: Jadvallarni bog'lash mantiqi. SQL JOIN sintaksisini to'liq qo'llaydi.

- `@fields`: DB ustunlarini Frontend Key'lariga map qilish (Alias).

- `@config`: Global sozlamalar (limit, offset, order).

### Complex Request Example:
```json
{
  "employee": {
    "@source": "personal[status: 'active', age: 25..45]",
    "@fields": {
      "id": "id",
      "full_name": "CONCAT(last_name_latin, ' ', first_name_latin)",
      "passport": "jshshir"
    },
    "organization": {
      "@source": "org",
      "@join": "INNER JOIN org ON personal.org_id = org.id",
      "@fields": {
        "name": "name_uz",
        "code": "code"
      }
    },
    "position_info": {
      "@source": "pos[rank_id: in (1, 2, 3)]",
      "@join": "LEFT JOIN pos ON personal.pos_id = pos.id",
      "@fields": {
        "title": "name_latin",
        "is_military": "is_military_rank"
      }
    }
  },
  "@config": {
    "limit": 15,
    "order": "personal.id DESC"
  }
}
```

## 3. Core Engine Architecture (Internal Logic)
   Kutubxona 4 ta asosiy komponentdan iborat bo'lishi kerak:

### 3.1. Lexer & Parser
JSON qiymatlarini (ayniqsa `@source` ichidagi stringlarni) o'qib, quyidagi elementlarga ajratadi:
- **Table Name**: `personal`
- **Conditions:** `[{field: "status", op: "eq", val: "active"}, ...]`
- **Tree Structure**: Obyektlar ierarxiyasi.

### 3.2. SQL Generator (Multi-Dialect)
Parserdan olingan natija asosida SQL yasaladi:
- **MySQL/PostgreSQL/SQLite** dialektlarini qo'llab-quvvatlash.
- **Automatic Aliasing:** Jadvallar to'qnashmasligi uchun `t1`, `t2`, `t3` aliaslarini generatsiya qilish.
- **Deep Joins:** Ierarxiyadagi har bir obyektni tegishli JOIN blokiga aylantirish.

### 3.3. Security & Validation (The Guard)
- **SQL Injection Prevention:** Barcha qiymatlar parametrli so'rov (`:p1`, `:p2`) ko'rinishida generatsiya qilinadi.
- **Function Whitelist:** Faqat `CONCAT`, `COUNT`, `DATE_FORMAT` kabi xavfsiz funksiyalarga ruxsat berish.
- **Table Whitelist:** Faqat ruxsat etilgan jadvallarga so'rov yuborish.

## 4. Universal Interoperability (Cross-Language Support)
   Kutubxona har qanday tilda ishlashi uchun quyidagi formatlarda build qilinadi:

| Platform               | Format       | Integration Method                     |
|------------------------|--------------|----------------------------------------|
| **PHP (Yii2/Laravel)** | `.so`/`.dll` | PHP FFI yoki Native Extension          |
| **Java (Spring)**      | `.jar` (JNI) | Java Native Interface / Project Panama |
| **Node.js**            | `.wasm`      | WebAssembly Runtime                    |
| **Python**             | `.so`        | ctypes yoki cffi                       |

## 5. Output Specification
   Kutubxona chaqirilganda natija sifatida quyidagi obyektni qaytarishi kerak:
```rs
struct ParseResult {
    sql: String,        // Tayyor SQL string
    params: HashMap,    // :p1, :p2 kabi parametrlar va ularning qiymatlari
    structure: JSON     // Frontend uchun qaytishi kerak bo'lgan JSON formati (metadata)
}
```

```json
{
  "sql": "SELECT personal.id AS id, CONCAT(personal.last_name_latin, ' ', personal.first_name_latin) AS full_name, personal.jshshir AS passport, org.name_uz AS name, org.code AS code, pos.name_latin AS title, pos.is_military_rank AS is_military FROM personal INNER JOIN org ON personal.org_id = org.id LEFT JOIN pos ON personal.pos_id = pos.id WHERE personal.status = :p1 AND personal.age BETWEEN :p2 AND :p3 AND pos.rank_id IN (:p4, :p5, :p6) ORDER BY personal.id DESC LIMIT 15",
  
  "params": {
    "p1": "active",
    "p2": 25,
    "p3": 45,
    "p4": 1,
    "p5": 2,
    "p6": 3
  },

  "structure": {
    "employee": {
      "fields": ["id", "full_name", "passport"],
      "organization": {
        "fields": ["name", "code"]
      },
      "position_info": {
        "fields": ["title", "is_military"]
      }
    }
  }
}
```
  Oxirgi natija:
```json
{
  "status": "success",
  "data": [
    {
      "id": 101,
      "full_name": "Toshmatov Ali",
      "passport": "1234567",
      "organization": {
        "name": "IIV",
        "code": "001"
      },
      "position_info": {
        "title": "Katta inspektor",
        "is_military": 1
      }
    }
  ]
}
```