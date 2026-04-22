# UAQ Engine — To'liq Qo'llanma

> **Universal Adaptive Query Engine** — Rust tilida yozilgan, JSON so'rovlardan xavfsiz PostgreSQL so'rovlarini avtomatik generatsiya qiluvchi yuqori unumdorli kutubxona.

---

## Mundarija

- [Tizimga Umumiy Nazar](#tizimga-umumiy-nazar)
- [Backend Qo'llanmasi](#backend-qollanmasi)
  - [1. Integratsiya (FFI)](#1-integratsiya-ffi)
  - [2. Whitelist — Xavfsizlik Qatlami](#2-whitelist--xavfsizlik-qatlami)
  - [3. Relations — Avtomatik JOIN](#3-relations--avtomatik-join)
  - [4. Macros — Qayta Ishlatiluvchi Shablonlar](#4-macros--qayta-ishlatiluvchi-shablonlar)
  - [5. Tizim Introspeksiyasi (@info)](#5-tizim-introspeksiyasi-info)
  - [6. Xavfsizlik Modeli](#6-xavfsizlik-modeli)
- [Frontend Qo'llanmasi](#frontend-qollanmasi)
  - [1. So'rov Strukturasi](#1-sorov-strukturasi)
  - [2. @source — Manba va Filtrlar](#2-source--manba-va-filtrlar)
  - [3. @fields — Maydonlar](#3-fields--maydonlar)
  - [4. Ichma-ich So'rovlar](#4-ichma-ich-sorovlar)
  - [5. Maxsus Maydon Funksiyalari](#5-maxsus-maydon-funksiyalari)
  - [6. Amaliy Misollar](#6-amaliy-misollar)
- [Chiqish Formati](#chiqish-formati)
- [Cheklovlar va Xatolar](#cheklovlar-va-xatolar)
- [Fayl Ma'lumotlarini Base64 ga Aylantirish](#fayl-malumotlarini-base64-ga-aylantirish)
- [Barcha Integratsiya Tillari](#barcha-integratsiya-tillari)
- [Build va O'rnatish](#build-va-ornatish)
- [Best Practices](#best-practices)

---

## Tizimga Umumiy Nazar

```
Frontend JSON  →  uaq_parse()  →  SQL + params  →  PostgreSQL  →  JSON natija
```

1. **Frontend** deklarativ JSON yuboradi
2. **Backend middleware** `uaq_parse(json, whitelist, relations, macros)` chaqiradi
3. **UAQ Engine** xavfsiz, parametrlangan SQL qaytaradi
4. **Backend** tayyor SQL ni prepared statement orqali bazaga yuboradi

---

## Backend Qo'llanmasi

### 1. Integratsiya (FFI)

#### C API

```c
// SQL va params generatsiya
char* uaq_parse(
    const char* json_input,      // Frontenddan kelgan JSON
    const char* whitelist_json,  // Xavfsizlik konfiguratsiyasi
    const char* relations_json,  // Jadvallararo JOIN yo'llari
    const char* macros_json      // Ixtiyoriy (null bo'lishi mumkin)
);

// Bazadan kelgan JSON ichidagi fayl yo'llarini base64 ga o'girish
char* uaq_inject_base64_files(
    const char* json_result,
    const char* root_files_path,
    const char* trigger_prefix
);

// Xotirani tozalash — MAJBURIY!
void uaq_free_string(char* s);
```

> ⚠️ `uaq_parse()` va `uaq_inject_base64_files()` dan qaytgan har bir `char*` uchun `uaq_free_string()` chaqirish **majburiy**.

#### PHP

```php
$ffi = \FFI::cdef("
    char* uaq_parse(const char* json, const char* wl, const char* rels, const char* macros);
    char* uaq_inject_base64_files(const char* json, const char* path, const char* prefix);
    void  uaq_free_string(char* s);
", __DIR__ . '/libjson_to_sql.so');

$raw    = $ffi->uaq_parse($jsonInput, $whitelist, $relations, null);
$result = json_decode(\FFI::string($raw), true);
$ffi->uaq_free_string($raw);

if (!$result['isOk']) {
    throw new \RuntimeException($result['message']);
}

$stmt = $pdo->prepare($result['sql']);
$stmt->execute($result['params']);
$data = $stmt->fetchColumn();          // JSON string
```

#### Python

```python
import ctypes, json

lib = ctypes.CDLL('./libjson_to_sql.so')
lib.uaq_parse.restype          = ctypes.c_char_p
lib.uaq_free_string.argtypes   = [ctypes.c_char_p]

raw    = lib.uaq_parse(json_in.encode(), wl.encode(), rels.encode(), None)
result = json.loads(raw.decode())
lib.uaq_free_string(raw)
```

#### Node.js (ffi-napi)

```js
const ffi  = require('ffi-napi');
const ref  = require('ref-napi');

const lib = ffi.Library('./libjson_to_sql.so', {
  uaq_parse:         ['string', ['string','string','string','string']],
  uaq_free_string:   ['void',   ['string']],
});

const raw    = lib.uaq_parse(jsonInput, whitelist, relations, null);
const result = JSON.parse(raw);
lib.uaq_free_string(raw);
```

---

### 2. Whitelist — Xavfsizlik Qatlami

Whitelist frontendga **qaysi jadval va ustunlar ko'rinishi**ni belgilaydi. Format: `"haqiqiy_jadval:alias"`.

#### 2.1. Oddiy ruxsat ro'yxati

```json
{
  "employee:emp":                        ["id", "first_name", "last_name", "status", "birthday"],
  "employee_education:edu":              ["id", "status", "end_year", "diploma_type_name"],
  "shtat_department_basic:departmentBasic": ["*"]
}
```

- `["*"]` — barcha ustunlarga ruxsat (frontend haqiqiy ustun nomlarini ishlatadi)
- Alias (`emp`) — frontend faqat shu nom orqali murojaat qiladi; haqiqiy nom (`employee`) yashiriladi
- Whitelist'da yo'q jadval yoki ustundan `isOk: false` xatolik qaytaradi

#### 2.2. Mapping — Virtual va Qayta Nomlangan Maydonlar

Haqiqiy DB ustun nomlarini yashirish yoki SQL ifodalarini virtual maydon sifatida taqdim etish:

```json
{
  "employee:emp": {
    "id":        "id",
    "full_name": "CONCAT(last_name, ' ', first_name)",
    "birthDay":  "TO_CHAR(TO_TIMESTAMP(birthday), 'DD.MM.YYYY')",
    "age":       "EXTRACT(YEAR FROM AGE(TO_TIMESTAMP(birthday)))",
    "status":    "status"
  },
  "shtat_department_basic:departmentBasic": {
    "id":           "id",
    "name":         "name_uz",
    "status":       "status",
    "has_children": "EXISTS(SELECT 1 FROM shtat_department_basic WHERE parent_id = departmentBasic.id)",
    "is_active":    "CASE WHEN status = 1 THEN true ELSE false END"
  },
  "structure_organization:org": {
    "unique": "id",
    "title":  "name_uz",
    "active": "status"
  }
}
```

**Qanday ishlaydi:**

| Frontend yozadi                       | SQL da aylanadi                                          |
|---------------------------------------|----------------------------------------------------------|
| `emp.full_name`                       | `CONCAT(employee.last_name, ' ', employee.first_name)`   |
| `emp.birthDay`                        | `TO_CHAR(TO_TIMESTAMP(employee.birthday), 'DD.MM.YYYY')` |
| `org[active: 1]`                      | `WHERE structure_organization.status = :p1`              |
| `departmentBasic[has_children: true]` | `WHERE EXISTS(...) = :p1`                                |

> **Muhim:** Virtual maydonlar `@source` filtri sifatida ham ishlatiladi — frontend oddiy ustundek yozadi.

#### 2.3. Expression Tip Aniqlash (@info da ko'rinadi)

| SQL Ifoda boshlanishi         | Qaytariladigan tip  |
|-------------------------------|---------------------|
| `EXISTS(...)`, `NOT EXISTS`   | `boolean`           |
| `CASE WHEN ... THEN true`     | `boolean`           |
| `CONCAT(...)`, `TO_CHAR(...)` | `character varying` |
| `EXTRACT(...)`, `LENGTH(...)` | `numeric`           |
| `TO_TIMESTAMP(...)`, `NOW()`  | `timestamp`         |
| `COUNT(...)`, `SUM(...)`      | `numeric`           |
| Boshqalar                     | `expression`        |

---

### 3. Relations — Avtomatik JOIN

Jadvallar orasidagi bog'lanishni **bir marta** yozasiz — Engine kerak bo'lganda avtomatik JOIN quradi.

#### 3.1. Relation Formati

```
"alias1 OPERATOR alias2[:rel_nomi]": "@join @table ON @1.ustun = @2.ustun [AND ...]"
```

| Kalit operator | SQL JOIN turi | Izoh                    |
|----------------|---------------|-------------------------|
| `->`           | LEFT JOIN     | alias1 dan alias2 ga    |
| `<-`           | RIGHT JOIN    | alias1 dan alias2 ga    |
| `-><-`         | INNER JOIN    | ikki tomonlama majburiy |
| `<->`          | FULL JOIN     | ikki tomonlama to'liq   |

| Placeholder | Ma'nosi                                        |
|-------------|------------------------------------------------|
| `@join`     | Operatorga mos JOIN nomi (LEFT JOIN va h.k.)   |
| `@table`    | Child jadvalning haqiqiy nomi (AS alias bilan) |
| `@1`        | Kalitdagi birinchi (parent) alias              |
| `@2`        | Kalitdagi ikkinchi (child) alias               |

```json
{
  "emp->dept":                "@join @table ON @2.employee_id = @1.id",
  "dept->deptBasic":          "@join @table ON @2.id = @1.department_basic_id",
  "deptBasic<->org":          "@join @table ON @1.organization_id = @2.id",
  "deptBasic<->innerOrg":     "@join @table ON @1.command_organization_id = @2.id",
  "emp->education":           "@join @table ON @2.employee_id = @1.id AND @2.status = 1",
  "emp->positionBasic:admin": "@join @table ON @2.employee_id = @1.id AND @2.type = 'admin'"
}
```

#### 3.2. Bir Jadvalga Ikki Xil Aloqa (`:rel_nomi`)

Bir xil jadval juftligiga bir necha xil usulda ulanish kerak bo'lsa, relation nomiga `:suffix` qo'shing:

```json
{
  "emp->positionBasic":       "@join @table ON @2.employee_id = @1.id",
  "emp->positionBasic:admin": "@join @table ON @2.employee_id = @1.id AND @2.type = 'admin'"
}
```

Frontend `$rel: admin` deb aniq relation tanlaydi:
```json
{ "@source": "positionBasic[$rel: admin, status: 1]" }
```

#### 3.3. Bitta Haqiqiy Jadval — Ikki Alias

```json
// Whitelist
{
  "structure_organization:org":      { "id": "id", "name": "name_uz" },
  "structure_organization:innerOrg": { "id": "id", "name": "name_uz" }
}

// Relations
{
  "deptBasic->org":      "@join @table ON @1.viloyat_id = @2.id",
  "deptBasic->innerOrg": "@join @table ON @1.tuman_id = @2.id"
}
```

#### 3.4. Auto-Path — BFS Orqali Yo'l Topish

To'g'ridan-to'g'ri `emp → org` relation bo'lmasa ham Engine grafdan eng qisqa yo'lni topadi:

```
emp → dept → deptBasic → org   (3 oraliq jadval avtomatik JOIN qilinadi)
```

Frontend faqat `"@source": "org"` deb yozadi — oraliq jadvallar haqida bilishi shart emas.

---

### 4. Macros — Qayta Ishlatiluvchi Shablonlar

Tez-tez takrorlanadigan murakkab so'rovlarni oldindan tayyorlab, frontend tomonidan parameter bilan chaqirish mumkin.

#### Ta'rif (Backend)

```json
{
  "activeEmployee": {
    "@source": "emp[status: 1]",
    "@fields": {
      "id":        "id",
      "full_name": "full_name",
      "birthDay":  "birthDay",
      "jshshir":   "jshshir"
    }
  },

  "currentPosition": {
    "@source": "departmentStaffPosition[status: 1, is_current: true]",
    "@fields": {
      "id":         "id",
      "begin_date": "start_time"
    },
    "ishJoyi": {
      "@source": "departmentBasic[status: 1]",
      "@flatten": true,
      "@fields":  ["*"]
    }
  }
}
```

#### Ishlatish (Frontend)

```json
// To'g'ridan-to'g'ri
{ "@data[]": { "@source": "activeEmployee" } }

// Qo'shimcha filtr bilan
{ "@data[]": { "@source": "activeEmployee[$limit: 20, $order: id DESC]" } }

// Macro'ni kengaytirib
{
  "@data": {
    "@source": "activeEmployee[id: 42]",
    "@fields": {
      "id":        "id",
      "full_name": "full_name"
    },
    "positions[]": {
      "@source": "currentPosition[$limit: 5]"
    }
  }
}
```

---

### 5. Tizim Introspeksiyasi (@info)

Frontend uchun qaysi jadvallar, maydonlar va tiplar mavjudligini ko'rsatadi.

```json
{ "@info": ["@tables", "@relations"] }
```

SQL ni bazaga yuborib natija olasiz:
```json
{
  "tables": {
    "emp": {
      "id":           "integer",
      "full_name":    "character varying",
      "birthDay":     "character varying",
      "has_children": "boolean"
    }
  },
  "relations": ["emp->dept", "dept->deptBasic", "deptBasic->org"]
}
```

---

### 6. Xavfsizlik Modeli

UAQ Engine ko'p qatlamli himoya tizimiga ega. Backend **whitelist berishni majburiy** deb hisoblash kerak — whitelist bo'lmasa jadval/ustun mavjudligi tekshirilmaydi.

| Qatlam                 | Himoya                                                                                                      |
|------------------------|-------------------------------------------------------------------------------------------------------------|
| **Parameterization**   | Barcha filtr qiymatlari `:p1`, `:p2` — SQL injection imkonsiz                                               |
| **Whitelist**          | Frontend faqat ruxsat etilgan jadval/ustunlarga murojaat qiladi                                             |
| **Global tahdid**      | `DROP`, `DELETE`, `UPDATE`, `INSERT`, `SELECT`, `UNION`, `--`, `/* */`, `;` qat'iy bloklanadi               |
| **Funksiya ro'yxati**  | `@fields` da faqat ruxsat etilgan funksiyalar: `CONCAT`, `TO_CHAR`, `COALESCE`, `CASE WHEN`, `CAST` va h.k. |
| **$order validatsiya** | Parse vaqtida `^[a-zA-Z0-9_\.]+(\s+(ASC\|DESC))?$` tekshiruvi                                               |
| **$join validatsiya**  | Faqat: `left`, `right`, `inner`, `full`, `cross` (va kalit ekvivalentlari)                                  |
| **$limit cheklovi**    | Maksimal `10 000` — katta so'rovlar avtomatik kesib qo'yiladi                                               |
| **$rel validatsiya**   | Faqat `^[a-zA-Z0-9_]+$` formatda                                                                            |
| **Alias majburiyati**  | Whitelist'da alias berilgan jadvalni haqiqiy nomi bilan chaqirish rad etiladi                               |

---

## Frontend Qo'llanmasi

Frontend dasturchi **SQL bilmaydi** — faqat qaysi ma'lumot kerakligini JSON orqali bildiradi.

### 1. So'rov Strukturasi

Root kalit uchta variantdan biri:

| Kalit     | Qaytariladi                                 |
|-----------|---------------------------------------------|
| `@data`   | Bitta obyekt `{...}` (LIMIT 1)              |
| `@data[]` | Massiv `[{...}, ...]`                       |
| `@info`   | Jadval strukturasi va relation ro'yxati     |

```json
{
  "@data[]": {                          ← massiv qaytaradi
    "@source": "emp[status: 1]",        ← manba + filtrlar
    "@fields": { "id": "id" }           ← qaytariladigan maydonlar
  }
}
```

---

### 2. @source — Manba va Filtrlar

```
alias[maydon: qiymat, $limit: N, $offset: N, $order: col DIR, $join: tur, $rel: nom]
```

#### 2.1. Filtr Operatorlari

| Operator | SQL     | Misol                  |
|----------|---------|------------------------|
| `:`      | `=`     | `status: 1`            |
| `!:`     | `!=`    | `type: !: 0`           |
| `>`      | `>`     | `age: > 18`            |
| `<`      | `<`     | `age: < 65`            |
| `..`     | BETWEEN | `id: 1000..2000`       |
| `~`      | LIKE    | `full_name: ~ Aliyev%` |
| `in`     | IN      | `rank: in (1, 2, 3)`   |

```json
"@source": "emp[status: 1, id: 61480..66580, full_name: ~ Ali%, rank: in (1, 2, 3), $limit: 20]"
```

#### 2.2. Konfiguratsiya Parametrlari

| Parametr  | Misol             | Izoh                                             |
|-----------|-------------------|--------------------------------------------------|
| `$limit`  | `$limit: 20`      | Qaytariladigan maksimal qatorlar (max: 10 000)   |
| `$offset` | `$offset: 40`     | Skip — sahifalash uchun                          |
| `$order`  | `$order: id DESC` | `ustun_nomi [ASC\|DESC]` formatida               |
| `$join`   | `$join: inner`    | JOIN turini qo'lda o'zgartirish                  |
| `$rel`    | `$rel: emp_admin` | Bir necha relation bo'lganda aniq birini tanlash |

**`$join` qiymatlari:**

| Qiymat              | JOIN turi  |
|---------------------|------------|
| `left` yoki `->`    | LEFT JOIN  |
| `right` yoki `<-`   | RIGHT JOIN |
| `inner` yoki `-><-` | INNER JOIN |
| `full` yoki `<->`   | FULL JOIN  |
| `cross`             | CROSS JOIN |

---

### 3. @fields — Maydonlar

#### 3.1. Massiv formatida (oddiy)

```json
"@fields": ["id", "full_name", "status"]
```

Barcha ruxsat etilgan maydonlar:
```json
"@fields": ["*"]
```

#### 3.2. Obyekt formatida (qayta nomlash va ifodalar)

```json
"@fields": {
  "employee_id":   "id",
  "ism_sharif":    "full_name",
  "tug_san":       "birthDay",
  "formatlangan":  "TO_CHAR(TO_TIMESTAMP(birthday), 'DD.MM.YYYY')",
  "yosh":          "EXTRACT(YEAR FROM AGE(TO_TIMESTAMP(birthday)))"
}
```

`"chiqish_kaliti": "manba_maydon_yoki_ifoda"` — chiqish JSON da `chiqish_kaliti` nomida, SQL da esa ifoda qiymatida ko'rinadi.

Ruxsat etilgan SQL funksiyalar: `CONCAT`, `SUBSTR`, `UPPER`, `LOWER`, `TRIM`, `LENGTH`, `COALESCE`, `NULLIF`, `TO_CHAR`, `TO_TIMESTAMP`, `TO_DATE`, `NOW`, `EXTRACT`, `AGE`, `CAST`, `ROUND`, `CEIL`, `FLOOR`, `ABS`, `SPLIT_PART`, `CASE WHEN ... END` va boshqalar.

---

### 4. Ichma-ich So'rovlar

#### 4.1. Bitta Ichki Obyekt (One-to-One)

```json
{
  "@data[]": {
    "@source": "emp[status: 1, $limit: 10]",
    "@fields": { "id": "id", "full_name": "full_name" },

    "department": {
      "@source": "departmentBasic[status: 1]",
      "@fields": { "id": "id", "name": "name" }
    }
  }
}
```

Natija:
```json
[{
  "id": 1, "full_name": "Aliyev Ali",
  "department": { "id": 5, "name": "Moliya bo'limi" }
}]
```

#### 4.2. Ichki Massiv (One-to-Many) — `[]` suffiks

```json
{
  "@data": {
    "@source": "emp[id: 42]",
    "@fields": { "id": "id", "full_name": "full_name" },

    "positions[]": {
      "@source": "departmentStaffPosition[status: 1, is_current: true, $limit: 5]",
      "@fields": { "id": "id", "begin_date": "start_time" }
    },

    "educations[]": {
      "@source": "education[$limit: 10, $order: id DESC]",
      "@fields": { "diploma_type": "diploma_type_name" }
    }
  }
}
```

#### 4.3. @flatten — Maydonlarni Ota Nodega Birlashtirish

```json
"degree": {
  "@source": "militaryDegree[current_degree: 1]",
  "@flatten": true,
  "@fields": { "degree_name": "name_uz", "degree_date": "given_date" }
}
```

`@flatten: true` bilan `degree` obyekti o'rniga uning maydonlari to'g'ridan-to'g'ri ota nodega qo'shiladi:

```
Flattensiz: { ..., "degree": { "degree_name": "Mayor", "degree_date": "2020-01-01" } }
Flattenli:  { ..., "degree_name": "Mayor", "degree_date": "2020-01-01" }
```

---

### 5. Maxsus Maydon Funksiyalari

Bu funksiyalar `@fields` ichida ishlatiladi va Engine tomonidan maxsus SQL ga aylantiriladi.

---

#### 5.1. `parents()` — Ierarxik Yo'l (Breadcrumb)

Joriy nodening o'zi va barcha ajdodlarini root dan boshlab tartibli qaytaradi.

```
parents(parent_ustun, id_ustun, maydonlar)
```

**4 ta format:**

```json
"@fields": {
  "dep_path":  "parents(parent_id, id, [name])",
  "dep_multi": "parents(parent_id, id, [name, code])",
  "dep_obj":   "parents(parent_id, id, {nn: name})",
  "dep_str":   "parents(parent_id, id, name)"
}
```

| Sintaksis          | Natija formati                                        |
|--------------------|-------------------------------------------------------|
| `[col]`            | `[{"col": "..."}, ...]` — JSON massiv                 |
| `[col1, col2]`     | `[{"col1": "...", "col2": "..."}, ...]`               |
| `{key: col}`       | `[{"key": "..."}, ...]` — custom kalit nomi           |
| `{k1: c1, k2: c2}` | `[{"k1": "...", "k2": "..."}, ...]`                   |
| `col`              | `"root, ..., joriy"` — vergul bilan ajratilgan string |

**Natija tartibi:** root birinchi → joriy node oxirida (breadcrumb).

**Misol:**
```json
{
  "@data[]": {
    "@source": "emp[status: 1, $limit: 10]",
    "@fields": { "id": "id", "full_name": "full_name" },

    "department": {
      "@source": "departmentBasic",
      "@fields": {
        "id":       "id",
        "name":     "name",
        "dep_path": "parents(parent_id, id, [name])",
        "dep_str":  "parents(parent_id, id, name)"
      }
    }
  }
}
```

Natija:
```json
{
  "department": {
    "id": 13743,
    "name": "Ikkinchi guruh",
    "dep_path": [
      { "name": "Respublika boshqarmasi" },
      { "name": "Hududiy boshqarma" },
      { "name": "Tuman bo'limi" },
      { "name": "Tarkibiy bo'linma" }
    ],
    "dep_str": "Respublika boshqarmasi, Hududiy boshqarma, Tuman bo'limi, Tarkibiy bo'linma"
  }
}
```

> ⚠️ `parents()` uchun `parent_ustun` va `id_ustun` whitelist'da bo'lishi shart.
> Tsiklli ma'lumotlarda 50 daraja chegarasi avtomatik qo'llanadi.

---

#### 5.2. Lokal Aggregat Funksiyalar

Agar bir node'ning **barcha** `@fields` qiymatlari aggregate funksiya bo'lsa va `@source` filtrlarida filtrsiz bo'lsa, Engine JOIN qilmay, har biri mustaqil correlated subquery sifatida generatsiya qiladi — natijalar to'g'ri va samarali.

```json
"education_data": {
  "@source": "education",
  "@fields": {
    "jami":          "count(*)",
    "oxirgi_yil":    "max(end_year)",
    "eng_erta":      "min(end_year)",
    "faollar":       "count([status: 1])",
    "faol_id_yig":   "sum([status: 1].id)",
    "o_rtacha_yil":  "avg([status: 1].end_year)"
  }
}
```

**Funksiyalar va sintaksis:**

| Funksiya          | Sintaksis                | Ma'nosi                         |
|-------------------|--------------------------|---------------------------------|
| `count(*)`        | `count(*)`               | Barcha qatorlar soni            |
| `count([f: v])`   | `count([status: 1])`     | Filtrlangan qatorlar soni       |
| `max(col)`        | `max(end_year)`          | Maksimal qiymat                 |
| `min(col)`        | `min(end_year)`          | Minimal qiymat                  |
| `sum([f: v].col)` | `sum([status: 1].id)`    | Filtrlangan qatorlar yig'indisi |
| `avg([f: v].col)` | `avg([status: 1].score)` | Filtrlangan qatorlar o'rtachasi |

**Filter sintaksisi `[field: value]`** — `@source` filtri bilan bir xil operatorlar:

```json
"count([status: 1])"             ← status = 1
"count([year: > 2020])"          ← year > 2020
"sum([type: in (1, 2)].score)"   ← type IN (1, 2) bo'lganda score yig'indisi
"max([active: !: 0].salary)"     ← active != 0 bo'lganda maksimal salary
```

> ⚠️ Lokal aggregat faqat **bitta darajada** ishlaydi: joriy node ning ota jadvali (`@data` ning manba jadvali) bilan bog'lanadi. Lokal aggregat node'da `@source` filtri bo'lsa, u JOIN qilinadi — barcha maydonlar aggregate bo'lishi talab qilinmaydi.

---


### 6. Amaliy Misollar

#### Misol 1: Ro'yxat + Sahifalash

```json
{
  "@data[]": {
    "@source": "emp[status: 1, $limit: 20, $offset: 0, $order: id DESC]",
    "@fields": ["id", "full_name", "jshshir", "birthDay"]
  }
}
```

#### Misol 2: Bitta Xodim — To'liq Ma'lumot

```json
{
  "@data": {
    "@source": "emp[status: 1, id: 42]",
    "@fields": {
      "id":        "id",
      "full_name": "full_name",
      "birthDay":  "birthDay",
      "jshshir":   "jshshir"
    },

    "boshqarma": {
      "@source": "org[status: 1]",
      "@flatten": true,
      "@fields":  { "viloyat_name": "title" }
    },

    "lavozim": {
      "@source": "departmentStaffPosition[status: 1, is_current: true]",
      "@flatten": true,
      "@fields":  { "begin_date": "start_time" },

      "bo_lim": {
        "@source": "departmentBasic[status: 1]",
        "@flatten": true,
        "@fields":  { "bo_lim_nomi": "name", "dep_path": "parents(parent_id, id, [name])" }
      }
    },

    "ta_limlar[]": {
      "@source": "education[$limit: 5, $order: id DESC]",
      "@fields":  { "diploma_turi": "diploma_type_name", "yil": "end_year" }
    },

    "statistika": {
      "@source": "education",
      "@fields":  {
        "jami_ta_lim":  "count(*)",
        "so_nggi_yil":  "max(end_year)",
        "faol_soni":    "count([status: 1])"
      }
    }
  }
}
```

#### Misol 3: Ierarxik Bo'limlar

```json
{
  "@data[]": {
    "@source": "emp[status: 1, id: 61480..66580, $limit: 10]",
    "@fields": { "id": "id", "full_name": "full_name" },

    "department": {
      "@source": "departmentBasic",
      "@fields": {
        "id":       "id",
        "name":     "name",
        "dep_path": "parents(parent_id, id, [name])",
        "dep_obj":  "parents(parent_id, id, {title: name})",
        "dep_str":  "parents(parent_id, id, name)"
      }
    }
  }
}
```

#### Misol 4: Virtual Maydon Bilan Filtr

```json
{
  "@data[]": {
    "@source": "departmentBasic[status: 1, has_children: true, $limit: 20]",
    "@fields": {
      "id":           "id",
      "name":         "name",
      "has_children": "has_children"
    }
  }
}
```

#### Misol 5: @info — Tizim Strukturasi

```json
{ "@info": ["@tables", "@relations"] }
```

---

## Chiqish Formati

### Muvaffaqiyatli natija

```json
{
  "isOk":    true,
  "sql":     "SELECT COALESCE(json_agg(t.uaq_data), '[]'::json) FROM (...) t",
  "params":  { "p1": 1, "p2": "Aliyev%", "p3": 20 },
  "message": "success"
}
```

### Xatolik natijasi

```json
{
  "isOk":    false,
  "sql":     null,
  "params":  null,
  "message": "Generation Error: Column 'password' does not exist in table 'emp'"
}
```

### @info natijasi

```json
{
  "isOk":      true,
  "sql":       "WITH input_json AS (...) SELECT jsonb_build_object(...)",
  "message":   "info",
  "relations": ["emp->dept", "dept->deptBasic"]
}
```

SQL ni bazaga yuborib natija olasiz:
```json
{
  "tables": {
    "emp":             { "id": "integer", "full_name": "character varying", "birthDay": "character varying" },
    "departmentBasic": { "id": "integer", "name": "character varying",     "has_children": "boolean" }
  },
  "relations": ["emp->dept", "dept->deptBasic", "deptBasic->org"]
}
```

---

## Cheklovlar va Xatolar

### Xavfsizlik sabab bloklanadigan so'rovlar

| Nima             | Sabab                                          |
|------------------|------------------------------------------------|
| `SELECT`         | Subquery injection oldini olish                |
| `DROP`, `DELETE` | Strukturaviy manipulyatsiya                    |
| `UNION`          | Ma'lumot o'g'irlash vektori                    |
| `--`, `/* */`    | Comment injection                              |
| `;`              | Ko'p-so'rov injection                          |
| `EXEC`, `COPY`   | Tizim darajasidagi xavfli operatorlar          |

### Umumiy xatolar va yechimlari

| Xato xabari                                     | Sabab                                         | Yechim                                          |
|-------------------------------------------------|-----------------------------------------------|-------------------------------------------------|
| `Table 'X' does not exist`                      | Whitelist'da jadval yo'q yoki noto'g'ri alias | Whitelist'ga qo'shing yoki alias tekshiring     |
| `Column 'X' does not exist in table 'Y'`        | Ustun whitelist'da yo'q                       | Whitelist'ga ustunni qo'shing                   |
| `No connection for A->B`                        | Jadvallar orasida relation yo'q               | Relations'ga qo'shing yoki auto-path tekshiring |
| `Unsafe or unsupported function call: X`        | Ruxsat etilmagan SQL funksiyasi               | Faqat ruxsat etilgan funksiyalardan foydalaning |
| `Forbidden SQL operation detected: SELECT`      | @fields da SELECT ishlatilgan                 | Native SQL funksiyalar bilan almashtiring       |
| `parents() string format supports only 1 field` | String formatda bir necha ustun               | `[col1, col2]` yoki `{k:col}` ishlatilsin       |

### Chegaralar

| Parametr               | Chegara    | Izoh                                            |
|------------------------|------------|-------------------------------------------------|
| `$limit`               | max 10 000 | Kattaroq qiymat avtomatik 10 000 ga tushiriladi |
| `$offset`              | max 10 000 | `$limit` bilan bir xil qoida                    |
| `parents()` chuqurligi | 50         | Tsiklli ma'lumotlarga qarshi himoya             |

---

## Fayl Ma'lumotlarini Base64 ga Aylantirish

Bazadan kelgan JSON ichidagi fayl yo'llarini (masalan `/uploads/photo.jpg`) to'g'ridan-to'g'ri Base64 data URI ga o'giradi. Bitta API so'rovda ham ma'lumot, ham fayl tarkibini qaytarish uchun ishlatiladi.

### API

```c
char* uaq_inject_base64_files(
    const char* json_result,     // Bazadan kelgan JSON string
    const char* root_files_path, // Fayllar joylashgan asosiy papka, masalan "/var/www/project"
    const char* trigger_prefix   // Qaysi qiymatlarni o'girish kerak, masalan "/uploads/"
);
```

- `json_result` — PostgreSQL dan kelgan JSON string (ichida fayl yo'llari bor)
- `root_files_path` — serverda fayllar joylashgan papka (`/var/www/project`)
- `trigger_prefix` — shu prefiksdagi qiymatlarni base64 ga o'giradi (`/uploads/`)

### PHP misoli

```php
// 1. UAQ orqali SQL va params oling
$raw    = $ffi->uaq_parse($jsonInput, $whitelist, $relations, null);
$result = json_decode(\FFI::string($raw), true);
$ffi->uaq_free_string($raw);

// 2. Bazaga yuboring
$stmt = $pdo->prepare($result['sql']);
$stmt->execute($result['params']);
$dbJson = $stmt->fetchColumn();   // PostgreSQL json_agg natijasi

// 3. Fayl yo'llarini base64 ga o'girish
$raw2   = $ffi->uaq_inject_base64_files($dbJson, '/var/www/project', '/uploads/');
$final  = json_decode(\FFI::string($raw2), true);
$ffi->uaq_free_string($raw2);
```

### Natija

```
Avval:  { "photo": "/uploads/pasport.jpg" }
Keyin:  { "photo": "data:image/jpeg;base64,/9j/4AAQSkZJRgAB..." }
```

Qo'llab-quvvatlanadigan MIME turlar:

| Kengaytma     | MIME                       |
|---------------|----------------------------|
| `jpg`, `jpeg` | `image/jpeg`               |
| `png`         | `image/png`                |
| `gif`         | `image/gif`                |
| `webp`        | `image/webp`               |
| `svg`         | `image/svg+xml`            |
| `pdf`         | `application/pdf`          |
| `mp4`         | `video/mp4`                |
| Boshqa        | `application/octet-stream` |

> ⚠️ **Qachon ishlatish kerak:** Bitta rekord batafsil ko'rinishida (bitta xodim kartasi, bitta hujjat).
> **Qachon ishlatmaslik kerak:** Ro'yxat endpointlarida — Base64 hajmni ~33% kattalashtiradi.

---

## Barcha Integratsiya Tillari

### PHP (FFI)

```php
<?php
class UAQEngine
{
    private \FFI $ffi;

    public function __construct(string $soPath)
    {
        $this->ffi = \FFI::cdef("
            char* uaq_parse(const char* json, const char* wl, const char* rels, const char* macros);
            char* uaq_inject_base64_files(const char* json, const char* path, const char* prefix);
            void  uaq_free_string(char* s);
        ", $soPath);
    }

    public function parse(string $json, string $whitelist, string $relations, ?string $macros = null): array
    {
        $raw    = $this->ffi->uaq_parse($json, $whitelist, $relations, $macros);
        $result = json_decode(\FFI::string($raw), true);
        $this->ffi->uaq_free_string($raw);
        return $result;
    }

    public function injectFiles(string $jsonResult, string $rootPath, string $prefix): mixed
    {
        $raw    = $this->ffi->uaq_inject_base64_files($jsonResult, $rootPath, $prefix);
        $result = json_decode(\FFI::string($raw), true);
        $this->ffi->uaq_free_string($raw);
        return $result;
    }
}

// Ishlatish
$uaq    = new UAQEngine(__DIR__ . '/libjson_to_sql.so');
$result = $uaq->parse($jsonInput, $whitelist, $relations);

if (!$result['isOk']) {
    http_response_code(400);
    echo json_encode(['error' => $result['message']]);
    exit;
}

$stmt = $pdo->prepare($result['sql']);
$stmt->execute($result['params']);
$data = $stmt->fetchColumn();    // JSON string (PostgreSQL json_agg)
echo $data;
```

---

### Python (ctypes)

```python
import ctypes
import json
from typing import Optional

class UAQEngine:
    def __init__(self, so_path: str):
        self.lib = ctypes.CDLL(so_path)
        self.lib.uaq_parse.restype           = ctypes.c_char_p
        self.lib.uaq_parse.argtypes          = [ctypes.c_char_p] * 4
        self.lib.uaq_inject_base64_files.restype  = ctypes.c_char_p
        self.lib.uaq_inject_base64_files.argtypes = [ctypes.c_char_p] * 3
        self.lib.uaq_free_string.argtypes    = [ctypes.c_char_p]

    def parse(self, json_input: str, whitelist: str, relations: str,
              macros: Optional[str] = None) -> dict:
        raw = self.lib.uaq_parse(
            json_input.encode(),
            whitelist.encode(),
            relations.encode(),
            macros.encode() if macros else None
        )
        result = json.loads(raw.decode())
        self.lib.uaq_free_string(raw)
        return result

    def inject_files(self, json_result: str, root_path: str, prefix: str) -> dict:
        raw = self.lib.uaq_inject_base64_files(
            json_result.encode(),
            root_path.encode(),
            prefix.encode()
        )
        result = json.loads(raw.decode())
        self.lib.uaq_free_string(raw)
        return result

# Ishlatish
uaq    = UAQEngine('./libjson_to_sql.so')
result = uaq.parse(json_input, whitelist, relations)

if not result['isOk']:
    raise ValueError(result['message'])

# psycopg2 bilan
cur.execute(result['sql'], result['params'])
data = cur.fetchone()[0]    # JSON string
```

---

### Node.js (ffi-napi)

```js
const ffi  = require('ffi-napi');
const ref  = require('ref-napi');

const lib = ffi.Library('./libjson_to_sql.so', {
    uaq_parse: [
        'string',
        ['string', 'string', 'string', 'string']
    ],
    uaq_inject_base64_files: [
        'string',
        ['string', 'string', 'string']
    ],
    uaq_free_string: ['void', ['string']],
});

class UAQEngine {
    parse(jsonInput, whitelist, relations, macros = null) {
        const raw    = lib.uaq_parse(jsonInput, whitelist, relations, macros);
        const result = JSON.parse(raw);
        lib.uaq_free_string(raw);
        return result;
    }

    injectFiles(jsonResult, rootPath, prefix) {
        const raw    = lib.uaq_inject_base64_files(jsonResult, rootPath, prefix);
        const result = JSON.parse(raw);
        lib.uaq_free_string(raw);
        return result;
    }
}

// Ishlatish (Express.js)
const uaq = new UAQEngine();

app.post('/api/query', async (req, res) => {
    const result = uaq.parse(
        JSON.stringify(req.body),
        WHITELIST,
        RELATIONS
    );

    if (!result.isOk) {
        return res.status(400).json({ error: result.message });
    }

    const { rows } = await pool.query(result.sql, Object.values(result.params));
    res.json(rows[0]);
});
```

---

### Java (JNI + JNA)

```java
// pom.xml: net.java.dev.jna:jna:5.13.0

import com.sun.jna.Library;
import com.sun.jna.Native;
import com.sun.jna.Pointer;

public class UAQEngine {

    interface UAQLib extends Library {
        UAQLib INSTANCE = Native.load("json_to_sql", UAQLib.class);
        Pointer uaq_parse(String json, String whitelist, String relations, String macros);
        Pointer uaq_inject_base64_files(String jsonResult, String rootPath, String prefix);
        void    uaq_free_string(Pointer s);
    }

    public Map<String, Object> parse(String json, String whitelist,
                                     String relations, String macros) {
        Pointer raw = UAQLib.INSTANCE.uaq_parse(json, whitelist, relations, macros);
        try {
            String resultStr = raw.getString(0);
            return new ObjectMapper().readValue(resultStr, Map.class);
        } finally {
            UAQLib.INSTANCE.uaq_free_string(raw);
        }
    }
}

// Ishlatish
UAQEngine uaq    = new UAQEngine();
Map result       = uaq.parse(jsonInput, whitelist, relations, null);

if (!(Boolean) result.get("isOk")) {
    throw new RuntimeException((String) result.get("message"));
}

String sql           = (String) result.get("sql");
Map<String, Object> params = (Map) result.get("params");

// Spring JDBC bilan
List<Map<String, Object>> data = namedJdbc.queryForList(sql, params);
```

---

### Go (cgo)

```go
package uaq

/*
#cgo LDFLAGS: -L. -ljson_to_sql
#include <stdlib.h>

char* uaq_parse(const char* json, const char* wl, const char* rels, const char* macros);
char* uaq_inject_base64_files(const char* json, const char* path, const char* prefix);
void  uaq_free_string(char* s);
*/
import "C"
import (
    "encoding/json"
    "unsafe"
)

type ParseResult struct {
    IsOk    bool                   `json:"isOk"`
    SQL     string                 `json:"sql"`
    Params  map[string]interface{} `json:"params"`
    Message string                 `json:"message"`
}

func Parse(jsonInput, whitelist, relations string, macros *string) (*ParseResult, error) {
    cJson      := C.CString(jsonInput)
    cWl        := C.CString(whitelist)
    cRels      := C.CString(relations)
    defer C.free(unsafe.Pointer(cJson))
    defer C.free(unsafe.Pointer(cWl))
    defer C.free(unsafe.Pointer(cRels))

    var cMacros *C.char
    if macros != nil {
        cMacros = C.CString(*macros)
        defer C.free(unsafe.Pointer(cMacros))
    }

    raw := C.uaq_parse(cJson, cWl, cRels, cMacros)
    defer C.uaq_free_string(raw)

    var result ParseResult
    if err := json.Unmarshal([]byte(C.GoString(raw)), &result); err != nil {
        return nil, err
    }
    return &result, nil
}
```

---

## Build va O'rnatish

### Kutubxonani Kompilyatsiya Qilish

```bash
# Reliz versiyasi (.so faylini olish)
cargo build --release

# Natija:
# target/release/libjson_to_sql.so   (Linux)
# target/release/libjson_to_sql.dylib (macOS)
# target/release/json_to_sql.dll      (Windows)
```

### .so Faylini Joylashtirish (Linux)

```bash
# Loyiha papkasiga nusxalash
cp target/release/libjson_to_sql.so /var/www/your-project/

# Yoki tizim kutubxona papkasiga
sudo cp target/release/libjson_to_sql.so /usr/local/lib/
sudo ldconfig
```

### PHP uchun php.ini Sozlamasi

```ini
; php.ini
extension=ffi
ffi.enable=true
```

### Testlarni Ishlatish

```bash
# Barcha testlar
cargo test

# Bitta test
cargo test test_parents_cte_generation -- --nocapture

# Reliz rejimida test
cargo test --release
```

---

## Best Practices

### Backend Uchun

**✅ To'g'ri: Whitelist'ni konfiguratsiya faylida saqlang**
```php
// config/whitelist.php
return json_encode([
    'employee:emp' => ['id', 'full_name', 'status', 'birthday'],
    'education:edu' => ['id', 'status', 'end_year', 'diploma_type_name'],
]);
```

**✅ To'g'ri: Relations'ni bir martalik konfiguratsiyada belgilang**
```php
// config/relations.php
return json_encode([
    'emp->edu'           => '@join @table ON @2.employee_id = @1.id',
    'emp->dept'          => '@join @table ON @2.employee_id = @1.id',
    'dept->deptBasic'    => '@join @table ON @2.id = @1.department_basic_id',
    'deptBasic<->org'    => '@join @table ON @1.organization_id = @2.id',
]);
```

**✅ To'g'ri: `uaq_free_string()` ni har doim chaqiring**
```php
$raw = $ffi->uaq_parse(...);
try {
    $result = json_decode(\FFI::string($raw), true);
} finally {
    $ffi->uaq_free_string($raw);   // try/finally — xato bo'lsa ham tozalanadi
}
```

**✅ To'g'ri: `isOk` ni tekshirib so'ng SQL'ni ishlatilg**
```php
if (!$result['isOk']) {
    // $result['message'] — xato sababi
    // Log yozib foydalanuvchiga umumiy xato qaytaring
    logger()->error('UAQ Error: ' . $result['message']);
    return response()->json(['error' => 'So\'rov noto\'g\'ri'], 400);
}
$stmt = $pdo->prepare($result['sql']);
$stmt->execute($result['params']);
```

**❌ Noto'g'ri: SQL ni string birlashtirish bilan ishlatish**
```php
// HECH QACHON BUNDAY QILMANG!
$db->query($result['sql']);                          // params yo'q — injection xavfi
$db->query($result['sql'] . " LIMIT " . $limit);    // qo'shimcha birlashtirish
```

**❌ Noto'g'ri: Whitelist'siz ishlatish (produksiyada)**
```php
// Test muhitida mumkin, lekin produksiyada XAVFLI
$result = $uaq->parse($json, '{}', $relations);  // bo'sh whitelist
```

---

### Frontend Uchun

**✅ To'g'ri: `@info` dan foydalanib mavjud maydonlarni bilib oling**
```json
{ "@info": ["@tables"] }
```
Keyin qaytgan `tables` dan qaysi alias va maydonlar borligini bilasiz.

**✅ To'g'ri: Filtrlarni to'g'ri formatda yozing**
```json
"@source": "emp[status: 1, id: in (1, 2, 3), name: ~ Ali%, age: > 18]"
```

**✅ To'g'ri: Lokal aggregatlardan foydalaning (JOIN oldini olish)**
```json
"stats": {
  "@source": "education",
  "@fields": {
    "total":  "count(*)",
    "active": "count([status: 1])",
    "last":   "max(end_year)"
  }
}
```

**✅ To'g'ri: `parents()` bilan ierarxik ma'lumot oling**
```json
"department": {
  "@source": "departmentBasic",
  "@fields": {
    "id":       "id",
    "name":     "name",
    "breadcrumb": "parents(parent_id, id, [name])",
    "path_str":   "parents(parent_id, id, name)"
  }
}
```

**❌ Noto'g'ri: Bir martada haddan ko'p `parents()` chaqiruvi**
```json
// Har bir parents() — alohida WITH RECURSIVE = bazaga alohida so'rov
// Kerak bo'lgan formatni tanlang, ikkalasini birga oling agar kerak bo'lsa
"dep_path":  "parents(parent_id, id, [name])",        // JSON array uchun
"dep_str":   "parents(parent_id, id, name)",           // String uchun — ikkala format kerak bo'lsa OK
"dep_extra": "parents(parent_id, id, [name, code])",   // Bu ortiqcha agar faqat name kerak bo'lsa
```

**❌ Noto'g'ri: Haddan ko'p ichma-ich massiv (LATERAL per-row)**
```json
// Har bir [] LATERAL subquery — katta to'plamda sekin
"@data[]": {
  "@source": "emp[$limit: 1000]",
  "positions[]":  { ... },     // 1000 lateral
  "educations[]": { ... },     // 1000 lateral
  "documents[]":  { ... }      // 1000 lateral — juda sekin!
}
```
Katta ro'yxatlar uchun avval asosiy ma'lumotni, keyin alohida so'rovda tafsilotni oling.

**✅ To'g'ri: Katta ro'yxatlar uchun $limit va $offset ishlating**
```json
"@source": "emp[status: 1, $limit: 50, $offset: 0, $order: id DESC]"
```

---

## Tezkor Murojaat (Cheat Sheet)

### @source to'liq sintaksisi
```
alias[
  maydon: qiymat           → =
  maydon: !: qiymat        → !=
  maydon: > qiymat         → >
  maydon: < qiymat         → <
  maydon: qiymat1..qiymat2 → BETWEEN
  maydon: ~ pattern%       → LIKE
  maydon: in (1, 2, 3)    → IN

  $limit:  N               → LIMIT N  (max 10 000)
  $offset: N               → OFFSET N
  $order:  col [ASC|DESC]  → ORDER BY
  $join:   left|right|inner|full|cross
  $rel:    relation_nomi   → aniq relation tanlash
]
```

### @fields to'liq sintaksisi
```json
{
  "chiqish_nomi": "ustun_nomi",
  "chiqish_nomi": "FUNKSIYA(ustun)",
  "chiqish_nomi": "CASE WHEN ... THEN ... END",
  "chiqish_nomi": "parents(parent_col, id_col, [name])",
  "chiqish_nomi": "parents(parent_col, id_col, {key: col})",
  "chiqish_nomi": "parents(parent_col, id_col, name)",
  "chiqish_nomi": "count(*)",
  "chiqish_nomi": "count([field: value])",
  "chiqish_nomi": "max(col)",
  "chiqish_nomi": "min(col)",
  "chiqish_nomi": "sum([field: value].col)",
  "chiqish_nomi": "avg([field: value].col)"
}
```

### Relation kalitlari
```
"A->B"         LEFT JOIN  A → B
"A<-B"         RIGHT JOIN A → B
"A-><-B"       INNER JOIN A ↔ B
"A<->B"        FULL JOIN  A ↔ B
"A->B:nom"     Nom bilan aniq relation
```

### Chiqish formati
```json
{ "isOk": true,  "sql": "...", "params": {"p1": 1}, "message": "success" }
{ "isOk": false, "sql": null,  "params": null,       "message": "Xato..." }
```
