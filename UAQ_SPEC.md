🛸 Project: Universal Adaptive JSON-to-SQL Engine (UAQ-Engine)
===========================

## 1. Executive Summary
   Ushbu loyiha frontenddan keladigan deklarativ JSON formatini tahlil qilib, xavfsiz, optimallashgan va parametrli PostgreSQL so'rovlarini generatsiya qiluvchi universal kutubxonani yaratishni maqsad qilgan. Kutubxona Rust tilida yoziladi va C-ABI orqali har qanday dasturlash tilida (PHP, Java, Node.js, Python) "native" tezlikda ishlash imkoniyatiga ega bo'ladi.

## 2. Frontend Request Specification (The Declarative JSON)
   Frontendchi backendga ma'lumotlar strukturasini va filtrlarini bitta ierarxik JSONda yuboradi. JSON ning o'zi to'g'ridan-to'g'ri root node hisoblanadi (`@data` o'rami kerak emas).

### Direktivalar:

| Direktiva  | Vazifasi                                                   | Shart |
|------------|------------------------------------------------------------|-------|
| `@source`  | Manba jadval, filtrlar, va konfiguratsiya                  | Ha    |
| `@fields`  | DB ustunlarini Frontend Key'lariga map qilish              | Ha    |
| `@flatten` | Bola node maydonlarini ota node ga birlashtirib yuborish   | Yo'q  |
| `@join`    | Qo'lda JOIN yozish (relations berilmagan holatda)          | Yo'q  |
| `[]`       | Kalit nomi oxirida — natija Array (ro'yxat) bo'lib qaytadi | Yo'q  |

### @source Sintaksisi:
```
table_name[field: value, field: operator value, $limit: N, $order: column DIR, $offset: N]
```

**Filtr operatorlari:**

| Operator | Ma'nosi          | Misol                   |
|----------|------------------|-------------------------|
| `:`      | Teng (eq)        | `status: 1`             |
| `!:`     | Teng emas (neq)  | `status: !: 0`          |
| `>`      | Katta            | `age: > 18`             |
| `<`      | Kichik           | `age: < 65`             |
| `..`     | Oraliq (between) | `id: 1..45`             |
| `~`      | O'xshash (like)  | `name: ~ Ali%`          |
| `in`     | Ro'yxatda (in)   | `rank_id: in (1, 2, 3)` |

**Maxsus konfiguratsiya parametrlari (`$` bilan boshlanadi):**

| Parametr  | Vazifasi                       | Misol                      |
|-----------|--------------------------------|----------------------------|
| `$limit`  | Qaytariladigan qatorlar soni   | `$limit: 50`               |
| `$offset` | Boshlang'ich o'tkazib yuborish | `$offset: 100`             |
| `$order`  | Tartiblash                     | `$order: id DESC` |

### Oddiy so'rov namunasi:
```json
{
  "@source": "emp[status: 1, $limit: 10, $order: id DESC]",
  "@fields": {
    "id": "id",
    "full_name": "CONCAT(last_name, ' ', first_name)"
  }
}
```

### Murakkab so'rov namunasi (JOIN + Flatten + List):
```json
{
  "@source": "emp[status: 1, id: 1..45, $limit: 2, $order: id DESC]",
  "@fields": {
    "id": "id",
    "full_name": "CONCAT(last_name, ' ', first_name)",
    "passport": "jshshir",
    "birthDay": "TO_CHAR(TO_TIMESTAMP(birthday), 'DD.MM.YYYY')"
  },
  "boshqarma": {
    "@source": "emp_rel_org[status: 1]",
    "@fields": {
      "begin_date": "TO_CHAR(TO_TIMESTAMP(created_at), 'DD.MM.YYYY')"
    },
    "0": {
      "@source": "org[status: 1]",
      "@flatten": true,
      "@fields": {
        "name": "name_uz"
      }
    }
  },
  "positions[]": {
    "@source": "department_staff_position[current_position: 1, $limit: 5, $order: id DESC]",
    "@fields": {
      "id": "id",
      "begin_date": "TO_CHAR(TO_TIMESTAMP(staff_position_start_time), 'DD.MM.YYYY')"
    },
    "0": {
      "@source": "shtat_staff_position_basic[status: 1]",
      "@flatten": true,
      "0": {
        "@source": "staff_position[status: 1]",
        "@flatten": true,
        "@fields": {
          "name": "name_uz"
        }
      }
    }
  },
  "degree": {
    "@source": "department_military_degree[current_degree: 1]",
    "@fields": {
      "id": "id",
      "degree_given_time": "TO_CHAR(TO_TIMESTAMP(degree_given_time), 'DD.MM.YYYY')"
    },
    "0": {
      "@source": "military_degree[status: 1]",
      "@flatten": true,
      "@fields": {
        "name": "name_uz"
      }
    }
  }
}
```

### `@flatten` Ishlash tartibi:
Agar bola node ga `"@flatten": true` berilsa, uning `@fields` lari ota node ning ob'ektiga birlashib (merge) ketadi. Natijada alohida ichki ob'ekt yaratilmaydi:
```
Flattensiz:  { "degree": { "id": 5, "info": { "name": "Kapitan" } } }
Flattenli:   { "degree": { "id": 5, "name": "Kapitan" } }
```

### `[]` (List/Array) Ishlash tartibi:
Kalit nomi oxiriga `[]` qo'shilsa, Engine uni `LEFT JOIN LATERAL` subquery orqali Array (`[{...}, {...}]`) sifatida qaytaradi. Bu One-to-Many (bitta xodimda bir nechta lavozim) holatlar uchun:
```
Oddiy node:  "position": { "id": 5, "name": "..." }        — bitta ob'ekt
List node:   "positions[]": [ {"id": 5}, {"id": 3}, ... ]   — massiv
```

---

## 3. Backend Parametrlari (FFI orqali Controller dan beriladi)

Engine `uaq_parse(json_input, whitelist_input, relations_input)` funksiyasi orqali 3 ta parametr qabul qiladi:

### 3.1. Whitelist Input (2nd Parameter)
Backend Controller tarafidan beriladi. Mijoz qaysi jadvalning qaysi ustunlarini o'qish imkoniga ega ekanligini qat'iy cheklaydi. Ro'yxatda bo'lmagan ustunlar (`password`, `token`) dan himoyalaydi.

**Format:** `"real_table_name:alias"` — alias ixtiyoriy. Agar alias berilsa, frontend `@source` da faqat shu aliasni yozadi. SQL da esa haqiqiy jadval nomi ishlatiladi.

```json
{
  "employee:emp": ["id", "last_name", "first_name", "jshshir", "status", "birthday", "organization_id"],
  "employee_rel_organization:emp_rel_org": ["*"],
  "structure_organization:org": ["id", "name_uz", "code"],
  "employee_department_staff_position:department_staff_position": ["*"],
  "manuals_staff_position:staff_position": ["id", "name_uz"],
  "employee_department_military_degree:department_military_degree": ["*"],
  "manuals_military_degree:military_degree": ["id", "name_uz"]
}
```
*Eslatma: `["*"]` — barcha ustunlarga ruxsat. Aliassiz ham yozish mumkin: `"employee": [...]`*

### 3.2. Relations Input (3rd Parameter — Auto-Join)
Jadvallarning o'zaro qanday JOIN bo'lishini aniqlaydi. Frontend `@join` yozishi shart emas — Engine avtomatik aniqlaydi.

**Placeholder lar:**

| Placeholder | Ma'nosi                             |
|-------------|-------------------------------------|
| `@1`        | Kalit ichidagi birinchi jadval nomi |
| `@2`        | Kalit ichidagi ikkinchi jadval nomi |
| `@table`    | Child (ulanuvchi) jadval nomi       |

**Yo'nalishli format (`->`):**
```json
{
  "employee->employee_rel_organization": "INNER JOIN @table ON @1.id = @2.employee_id AND @2.status = 1"
}
```

**Universal format (`<->`):**
Ikki yo'nalishda (A→B va B→A) ishlaydi. `@1` va `@2` kalitdagi tartibga mos keladi:
```json
{
  "employee_rel_organization<->employee": "INNER JOIN @table ON @1.employee_id = @2.id AND @1.current_organization = 1",
  "employee_rel_organization<->structure_organization": "INNER JOIN @table ON @1.organization_id = @2.id"
}
```

**Ustunlik tartibi:** `@join` (qo'lda) → `->` (yo'nalishli) → `<->` (universal)

---

## 4. Core Engine Architecture

### 4.1. Parser (`parser.rs`)
- JSON ni tahlil qilib `QueryNode` daraxtini tuzadi
- `@source` ichidan jadval nomi, filtrlar, `$limit`, `$order`, `$offset` ni ajratib oladi
- `[]` bilan tugaydigan kalit nomlarni `is_list: true` deb belgilaydi
- Eski format (`@data` + `@config`) ham qo'llab-quvvatlanadi (backward compatibility)

### 4.2. SQL Generator (`generator.rs`)
- `QueryNode` daraxtini PostgreSQL `json_build_object` va `json_agg` yordamida SQL ga aylantiradi
- Oddiy node lar uchun `JOIN` ishlatadi
- `is_list` node lar uchun `LEFT JOIN LATERAL` + ichki subquery yaratadi
- `@flatten` node larni ota node ga birlashtirib yuboradi
- `@fields` dagi oddiy ustunlarga avtomatik jadval aliasini (prefix) qo'shadi (Auto-Prefix)

### 4.3. Security & Validation (`guard.rs`)
- **SQL Injection Prevention:** Barcha qiymatlar parametrli (`:p1`, `:p2`) generatsiya qilinadi
- **Function Whitelist:** `CONCAT`, `TO_CHAR`, `TO_TIMESTAMP`, `CASE WHEN` kabi xavfsiz funksiyalarga ruxsat
- **Table/Column Whitelist:** Faqat ruxsat etilgan jadval va ustunlarga so'rov
- **Global Threat Detection:** `DROP`, `DELETE`, `--`, `/* */` kabi xavfli SQL belgilarini bloklash
- **Auto-Prefix:** Funksiyalar ichidagi oddiy ustun nomlariga avtomatik jadval aliasini qo'shib berish

---

## 5. Output Specification

### Muvaffaqiyatli natija:
```json
{
  "isOk": true,
  "sql": "SELECT COALESCE(json_agg(t.uaq_data), '[]'::json)\nFROM (\n  SELECT json_build_object(\n    'id', employee.id,\n    'full_name', CONCAT(employee.last_name, ' ', employee.first_name),\n    'boshqarma', json_build_object('name', structure_organization.name_uz),\n    'positions', positions_list.array_data\n  ) AS uaq_data\n  FROM employee AS employee\n  INNER JOIN employee_rel_organization ON ...\n  LEFT JOIN LATERAL (\n    SELECT COALESCE(json_agg(sub.item), '[]'::json) AS array_data\n    FROM (\n      SELECT json_build_object('id', ...) AS item\n      FROM employee_department_staff_position\n      WHERE employee.id = employee_department_staff_position.employee_id\n      ORDER BY employee_department_staff_position.id DESC\n      LIMIT 5\n    ) sub\n  ) positions_list ON true\n  WHERE employee.status = :p1\n  ORDER BY employee.id DESC\n  LIMIT 2\n) t",
  "params": {
    "p1": "1"
  },
  "structure": {
    "id": "employee.id",
    "full_name": "CONCAT(...)",
    "boshqarma": { "name": "structure_organization.name_uz" },
    "positions": { "_type": "array", "id": "..." }
  },
  "message": "success"
}
```

### Xatolik natijasi:
```json
{
  "isOk": false,
  "sql": null,
  "params": null,
  "structure": null,
  "message": "Generation Error: No relation defined for employee->unknown_table"
}
```

### Database dan qaytuvchi yakuniy natija:
```json
[
  {
    "id": 42,
    "full_name": "Majidov Botir",
    "passport": "11111111111111",
    "birthDay": "01.01.1993",
    "boshqarma": {
      "begin_date": "15.03.2020",
      "name": "Jizzax viloyat boshqarmasi"
    },
    "positions": [
      { "id": 105, "begin_date": "01.06.2023", "name": "Buxgalter" },
      { "id": 78, "begin_date": "15.01.2021", "name": "Yordamchi" }
    ],
    "degree": {
      "id": 12,
      "degree_given_time": "20.09.2022",
      "name": "Kapitan"
    }
  }
]
```

---

## 6. Universal Interoperability (Cross-Language Support)

| Platform               | Format       | Integration Method                     |
|------------------------|--------------|----------------------------------------|
| **PHP (Yii2/Laravel)** | `.so`/`.dll` | PHP FFI                                |
| **Java (Spring)**      | `.jar` (JNI) | Java Native Interface / Project Panama |
| **Node.js**            | `.wasm`      | WebAssembly Runtime                    |
| **Python**             | `.so`        | ctypes yoki cffi                       |

### PHP FFI namunasi:
```php
$ffi = \FFI::cdef("
    char* uaq_parse(const char* json_input, const char* whitelist, const char* relations);
    void uaq_free_string(char* s);
", "/path/to/libjson_to_sql.so");

$result = $ffi->uaq_parse($jsonString, $whitelistJson, $relationsJson);
$parsed = json_decode(\FFI::string($result));
$ffi->uaq_free_string($result);
```