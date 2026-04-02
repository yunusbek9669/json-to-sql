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
  "@data": {
    "@source": "personal[status: 'active', age: 25..45]",
    "@fields": {
      "id": "id",
      "fish": "CONCAT(last_name, ' ', first_name)",
      "passport": "jshshir"
    },
    "boshqarma": {
      "@source": "organization",
      "@fields": {
        "nomi": "name",
        "kod": "code"
      }
    },
    "lavozim_info": {
      "@source": "position[rank_id: in (1, 2, 3)]",
      "@flatten": true,
      "@fields": {
        "lavozim": "name",
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

### Whitelist Input Example (2nd Parameter)
Ushbu JSON Backend Controller tarafidan beriladi va mijoz (klient) qaysi jadvalning aynan qaysi ustunlarini o'qish imkoniga ega ekanligini qat'iy cheklaydi (SQL Injection va ruxsatsiz ma'lumotlar sirqib chiqishining oldini oladi). Hech kim belgilangan ro'yxatdan tasdiqlanmagan ustunlarni (masalan `password`, `token`) chaqira olmaydi.
```json
{
  "personal": ["id", "last_name", "first_name", "jshshir", "status", "age", "organization_id", "position_id"],
  "organization": ["id", "name", "code"],
  "department": ["id", "name"],
  "position": ["*"]
}
```
*Eslatma: Agar biror jadvalning barcha ustunlariga ruxsat bermoqchi bo'lsangiz `["*"]` yozib qo'yishingiz ham mumkin.*

### Relations Input Example (3rd Parameter - Auto-Join)
Ushbu xarita orqali siz jadvallarning o'zaro qay usulda JOIN bo'lishini Backendda saqlab qolasiz. UAQ Engine qaysi jadval nima bilan ulanishi kerak ekanligini ushbu Configuration dan avtomatik aniqlaydi.
```json
{
  "personal->organization": "INNER JOIN organization ON personal.organization_id = organization.id",
  "personal->position": "LEFT JOIN position ON personal.position_id = position.id AND position.deleted_at IS NULL",
  "organization<->department": "LEFT JOIN @table ON organization.id = department.organization_id"
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
  "isOk": true,
  "sql": "SELECT COALESCE(json_agg(t.uaq_data), '[]'::json) 
    FROM (
      SELECT json_build_object('fish', CONCAT(last_name, ' ', first_name), 'id', personal.id, 'passport', personal.jshshir, 'boshqarma', json_build_object('nomi', organization.name, 'kod', organization.code), 'lavozim_info', json_build_object('is_military', position.is_military_rank, 'lavozim', position.name)) AS uaq_data
      FROM personal AS personal
      INNER JOIN organization ON personal.organization_id = organization.id
      LEFT JOIN position ON personal.position_id = position.id
      WHERE personal.status = :p1 AND personal.age BETWEEN :p2 AND :p3 AND position.rank_id IN (:p4)
      ORDER BY personal.id DESC
      LIMIT 15
    ) t",
  "params": {
    "p1": "active",
    "p2": 25,
    "p3": 45,
    "p4": 1
  },
  "structure": {
    "@data": {
      "id": "personal.id",
      "fish": "CONCAT(last_name, ' ', first_name)",
      "passport": "personal.jshshir",
      "boshqarma": {"nomi": "organization.name", "kod": "organization.code"},
      "lavozim_info": {"lavozim": "position.name", "is_military": "position.is_military_rank"}
    }
  },
  "message": "success"
}
```
   Yakuniy natija:
```json
[
  {
      "id": 42,
      "fish": "Majidov Botir",
      "passport": "11111111111111",
      "boshqarma": {
          "nomi": "Jizzax viloyat boshqarmasi",
          "kod": "1001"
      },
      "lavozim_info": {
          "lavozim": "Buxgalter",
          "is_military": 0
      }
  }
]
```