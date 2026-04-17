# UAQ Engine — To'liq Dokumentatsiya

> **Universal Adaptive Query Engine** — Rust tilida yozilgan, JSON so'rovlardan xavfsiz PostgreSQL so'rovlarini generatsiya qiluvchi yuqori unumdorli kutubxona.

---

## Mundarija

- [Tizimga Umumiy Nazar](#tizimga-umumiy-nazar)
- [Backend Uchun Qo'llanma](#backend-uchun-qollanma)
  - [Integratsiya (FFI)](#1-integratsiya-ffi)
  - [Whitelist — Xavfsizlik Qatlami](#2-whitelist--xavfsizlik-qatlami)
  - [Relations — Avtomatik JOIN](#3-relations--avtomatik-join)
  - [Macros — Qayta Ishlatiluvchi Shablonlar](#4-macros--qayta-ishlatiluvchi-shablonlar)
  - [Tizim Introspeksiyasi (@info)](#5-tizim-introspeksiyasi-info)
- [Frontend Uchun Qo'llanma](#frontend-uchun-qollanma)
  - [So'rov Strukturasi](#1-sorov-strukturasi)
  - [Direktivalar](#2-direktivalar)
  - [Filtr Operatorlari](#3-filtr-operatorlari)
  - [Konfiguratsiya Parametrlari](#4-konfiguratsiya-parametrlari)
  - [Virtual Maydonlar Bilan Ishlash](#5-virtual-maydonlar-bilan-ishlash)
  - [So'rov Namunalari](#6-sorov-namunalari)
- [Chiqish Formati (Output)](#chiqish-formati-output)
- [Xavfsizlik](#xavfsizlik)
- [Qo'shimcha Imkoniyatlar](#qoshimcha-imkoniyatlar)
- [Ko'p Tilli Integratsiya](#kop-tilli-integratsiya)

---

## Tizimga Umumiy Nazar

```
Frontend (JSON) → UAQ Engine (Rust) → SQL + Params → PostgreSQL → JSON natija
```

**Qanday ishlaydi:**

1. **Frontend** — deklarativ JSON so'rov yuboradi (qaysi jadval, qaysi maydonlar, qanday filtr)
2. **Backend Middleware** — `uaq_parse()` funksiyasiga JSON + Whitelist + Relations + Macros beradi
3. **UAQ Engine** — xavfsiz, parametrlangan SQL va params qaytaradi
4. **Backend** — tayyor SQL ni PDO/prepared statement orqali bazaga yuboradi, natijani frontendga jo'natadi

**Asosiy afzalliklar:**

| Xususiyat          | Tavsif                                                                  |
|--------------------|-------------------------------------------------------------------------|
| 🔒 Xavfsizlik      | SQL injection imkonsiz, barcha qiymatlar parametrlashtirilgan           |
| ⚡ Tezlik           | Native Rust — ORM/Query Builder lardan 10–100x tez                      |
| 🔗 Auto-Join       | BFS orqali murakkab jadval yo'llarini avtomatik topish                  |
| 📐 Moslashuvchan   | Har qanday til bilan (PHP, Java, Node.js, Python) FFI integratsiya      |
| 🗂 Yagona Endpoint | Frontend uchun bitta API orqali istalgan strukturadagi ma'lumotni olish |

---

## Backend Uchun Qo'llanma

Backend dasturchining vazifasi:
1. Xavfsizlik qoidalarini (`whitelist`) belgilash
2. Jadvallar orasidagi aloqalarni (`relations`) yozish
3. Ixtiyoriy: qayta ishlatiluvchi shablonlar (`macros`) tayyorlash
4. Frontend so'rovini `uaq_parse()` ga berish va natijani bazaga yuborish

### 1. Integratsiya (FFI)

**C-Header:**
```c
char* uaq_parse(
    const char* json_input,      // Frontenddan kelgan JSON so'rov
    const char* whitelist_json,  // Xavfsizlik: qaysi jadval va ustunlarga ruxsat
    const char* relations_json,  // Jadvallararo JOIN yo'llari
    const char* macros_json      // Ixtiyoriy: qayta ishlatiluvchi shablonlar (null bo'lishi mumkin)
);

char* uaq_inject_base64_files(
    const char* json_result,     // Bazadan kelgan JSON natija
    const char* root_files_path, // Fayllar joylashgan asosiy papka
    const char* trigger_prefix   // Qaysi qiymatlarni base64 ga aylantirish
);

void uaq_free_string(char* s);   // Xotirani tozalash (MAJBURIY!)
```

> ⚠️ **Muhim:** `uaq_parse()` va `uaq_inject_base64_files()` dan qaytgan har bir string uchun `uaq_free_string()` chaqirish **majburiy** — aks holda memory leak yuzaga keladi.

**PHP (FFI) integratsiyasi:**
```php
$ffi = \FFI::cdef("
    char* uaq_parse(const char* json_input, const char* whitelist, const char* relations, const char* macros_input);
    char* uaq_inject_base64_files(const char* json_result, const char* root_files_path, const char* trigger_prefix);
    void uaq_free_string(char* s);
", __DIR__ . '/libjson_to_sql.so');

// 1. SQL va params olish
$raw    = $ffi->uaq_parse($jsonInput, $whitelist, $relations, null);
$result = json_decode(\FFI::string($raw), true);
$ffi->uaq_free_string($raw);

if (!$result['isOk']) {
    throw new Exception($result['message']);
}

// 2. PDO bilan bajarish (xavfsiz)
$stmt = $pdo->prepare($result['sql']);
$stmt->execute($result['params']);
$data = $stmt->fetchAll(PDO::FETCH_ASSOC);

// 3. Natijadagi fayl yo'llarini base64 ga aylantirish (ixtiyoriy)
$jsonStr = json_encode($data[0]['result']);
$raw2    = $ffi->uaq_inject_base64_files($jsonStr, '/var/www/project', '/uploads/');
$final   = json_decode(\FFI::string($raw2), true);
$ffi->uaq_free_string($raw2);
```

**Python (ctypes) integratsiyasi:**
```python
import ctypes, json

lib = ctypes.CDLL('./libjson_to_sql.so')
lib.uaq_parse.restype  = ctypes.c_char_p
lib.uaq_free_string.argtypes = [ctypes.c_char_p]

raw    = lib.uaq_parse(json_input.encode(), whitelist.encode(), relations.encode(), None)
result = json.loads(raw.decode())
lib.uaq_free_string(raw)
```

---

### 2. Whitelist — Xavfsizlik Qatlami

Whitelist — frontendga qaysi jadval va ustunlarni ko'rsatish mumkinligini belgilovchi asosiy xavfsizlik qatlami. Format: `"haqiqiy_jadval:alias"`.

#### 2.1. Oddiy ruxsat ro'yxati (Array formatida)

```json
{
  "employee:emp":          ["id", "first_name", "last_name", "status", "birthday", "jshshir"],
  "education:edu":         ["*"],
  "employee_education:ee": ["id", "status", "diploma_given_date", "end_year", "diploma_type_name"]
}
```

- `["*"]` — barcha ustunlarga ochiq ruxsat (frontend haqiqiy ustun nomlarini ishlatadi)
- Ro'yxatdagi ustunlar — faqat shu ustunlarga murojaat mumkin
- Alias (`emp`) — frontend faqat shu taxallus orqali jadvalga murojaat qiladi, haqiqiy jadval nomi (`employee`) yashiriladi

#### 2.2. Mapping va Virtual Maydonlar (Object formatida)

Frontend haqiqiy DB ustun nomlarini ko'rishi kerak bo'lmagan hollarda:

```json
{
  "employee:emp": {
    "id":        "id",
    "full_name": "CONCAT(last_name, ' ', first_name)",
    "jshshir":   "jshshir",
    "birthDay":  "TO_CHAR(TO_TIMESTAMP(birthday), 'DD.MM.YYYY')",
    "status":    "status"
  },
  "structure_organization:org": {
    "unique":  "id",
    "title":   "name_uz",
    "active":  "status"
  }
}
```

**Qanday ishlaydi:**

| Frontend yozadi  | SQL da aylanadi                                          |
|------------------|----------------------------------------------------------|
| `org[active: 1]` | `WHERE structure_organization.status = :p1`              |
| `emp.full_name`  | `CONCAT(employee.last_name, ' ', employee.first_name)`   |
| `emp.birthDay`   | `TO_CHAR(TO_TIMESTAMP(employee.birthday), 'DD.MM.YYYY')` |

#### 2.3. Virtual (Expression) Maydonlar

Haqiqiy jadvaldagi ustun emas, balki SQL ifodasi bo'lgan maydonlar. Masalan, `has_children`:

```json
{
  "shtat_department_basic:departmentBasic": {
    "id":           "id",
    "name":         "name_uz",
    "status":       "status",
    "has_children": "EXISTS(SELECT 1 FROM public.shtat_department_basic WHERE public.shtat_department_basic.parent_id = departmentBasic.department_basic)"
  }
}
```

> ✅ **Muhim:** Virtual maydonlar `@fields` da ishlatilgani kabi, `@source` filtrlari ichida ham bemalol ishlatilishi mumkin!
>
> ```json
> "@source": "departmentBasic[status: 1, has_children: true]"
> ```
>
> Engine avtomatik ravishda `has_children` ning SQL ifodasi ekanligini tushunadi va WHERE shartiga to'g'ri joylashtiradi. Frontend hech qanday farq sezmaydi.

**Expression avtomatik tip aniqlash** (`@info @tables` so'rovida):

| SQL Ifoda boshlanishi         | Qaytariladigan tip  |
|-------------------------------|---------------------|
| `EXISTS(...)`                 | `boolean`           |
| `NOT EXISTS(...)`             | `boolean`           |
| `TO_CHAR(...)`, `CONCAT(...)` | `character varying` |
| `EXTRACT(...)`, `LENGTH(...)` | `numeric`           |
| `TO_TIMESTAMP(...)`, `NOW()`  | `timestamp`         |
| `ARRAY_AGG(...)`              | `array`             |
| `JSONB_BUILD_OBJECT(...)`     | `json`              |
| `COUNT(...)`, `SUM(...)`      | `numeric`           |
| `CASE WHEN ... THEN true`     | `boolean`           |
| Boshqa murakkab ifodalar      | `expression`        |

---

### 3. Relations — Avtomatik JOIN

Jadvallar orasidagi bog'lanishni bir marta yozasiz — Engine zarur bo'lganda avtomatik JOIN quradi.

#### 3.1. Relation Formati

```
"alias1->alias2": "JOIN_TURI @table ON @1.ustun = @2.ustun"
```

**Kalit operatorlari:**

| Kalit  | JOIN turi  |
|--------|------------|
| `->`   | LEFT JOIN  |
| `<-`   | RIGHT JOIN |
| `-><-` | INNER JOIN |
| `<->`  | FULL JOIN  |

**Placeholder ma'nolari:**

| Placeholder | Ma'nosi                                                    |
|-------------|------------------------------------------------------------|
| `@join`     | Join turi (LEFT, RIGHT, INNER, FULL) nomi                  |
| `@table`    | Child jadvalning haqiqiy SQL nomi                          |
| `@1`        | Kalitdagi birinchi alias (parent) ning haqiqiy jadval nomi |
| `@2`        | Kalitdagi ikkinchi alias (child) ning haqiqiy jadval nomi  |

#### 3.2. Misol

```json
{
  "emp->empRelOrg":     "@join @table ON @2.employee_id = @1.id AND @2.status = 1",
  "empRelOrg->org":     "@join @table ON @2.id = @1.organization_id",
  "emp->dept":          "@join @table ON @2.employee_id = @1.id",
  "dept->deptBasic":    "@join @table ON @2.id = @1.department_basic_id",
  "deptBasic<->org":    "@join @table ON @1.organization_id = @2.id"
}
```

#### 3.3. Self-Referencing (Bitta Jadval — Ikki Alias)

Bitta haqiqiy jadvalga ikki xil maqsadda ulanish kerak bo'lganda:

**Whitelist:**
```json
{
  "structure_organization:org":      { "id": "id", "name": "name_uz" },
  "structure_organization:innerOrg": { "id": "id", "name": "name_uz" }
}
```

**Relations:**
```json
{
  "dept->org":      "@join @table ON @1.viloyat_id = @2.id",
  "dept->innerOrg": "@join @table ON @1.tuman_id = @2.id"
}
```

Frontend `org` va `innerOrg` ni alohida jadval kabi ishlatadi, ikkalasi ham `structure_organization` ga resolve bo'ladi.

#### 3.4. Auto-Path (BFS — Avtomatik Yo'l Topish)

`emp → org` to'g'ridan-to'g'ri relation bo'lmasa ham, Engine relations grafidan eng qisqa yo'lni topadi:

```
emp → dept → deptBasic → org
```

Engine barcha oraliq jadvallarni avtomatik JOIN qiladi. Frontend faqat:
```json
{ "@source": "org[status: 1]", "@fields": { "name": "title" } }
```
deb yozadi — xolos.

---

### 4. Macros — Qayta Ishlatiluvchi Shablonlar

Tez-tez chaqiriladigan murakkab so'rovlarni oldindan yozib, istalgan joyda ishlatish mumkin.

#### 4.1. Macro Ta'rifi (Backend)

```json
{
  "activeEmployee": {
    "@source": "emp[status: 1]",
    "@fields": {
      "id":       "id",
      "jshshir":  "jshshir",
      "full_name": "full_name",
      "birthDay":  "birthDay"
    }
  },

  "positionCteTable": {
    "@source": "departmentStaffPosition[status: 1]",
    "@fields": {
      "id":         "id",
      "is_current": "is_current",
      "start_time": "start_time"
    },
    "ishJoyi": {
      "@source": "departmentBasic[status: 1]",
      "@flatten": true,
      "@fields": ["*"]
    }
  }
}
```

#### 4.2. Frontend Macro Ishlatish

**To'g'ridan-to'g'ri macro chaqiruv:**
```json
{
  "@data[]": {
    "@source": "activeEmployee"
  }
}
```

**Qo'shimcha filtr va parametrlar bilan:**
```json
{
  "@data[]": {
    "@source": "activeEmployee[$limit: 10, $order: id DESC]"
  }
}
```

**Macro'ni kengaytirib, yangi maydonlar qo'shish:**
```json
{
  "@data": {
    "@source": "activeEmployee[id: 42]",
    "@fields": {
      "id":        "id",
      "full_name": "full_name",
      "positions[]": {
        "@source": "positionCteTable[is_current: true]"
      }
    }
  }
}
```

---

### 5. Tizim Introspeksiyasi (@info)

Frontend dasturlari uchun qaysi jadvallar, qaysi maydonlar va qanday tipda ekanligini bilish imkonini beradi.

**So'rov:**
```json
{ "@info": ["@tables", "@relations"] }
```

**`@tables`** — Whitelist + DB `information_schema` dan jadval va maydon tiplari:
```json
{
  "tables": {
    "emp": {
      "id":        "integer",
      "full_name": "character varying",
      "status":    "integer",
      "birthDay":  "character varying"
    },
    "departmentBasic": {
      "id":           "integer",
      "name":         "character varying",
      "status":       "integer",
      "has_children": "boolean"
    }
  },
  "relations": ["emp->org", "emp->dept", "dept->deptBasic"]
}
```

> ✅ Virtual (expression) maydonlar uchun `"expression"` emas, balki **haqiqiy qaytariladigan tip** ko'rsatiladi (`boolean`, `character varying`, `numeric`, va hokazo).

---

## Frontend Uchun Qo'llanma

Frontend dasturchi backend bilan kelishilgan whitelist alias nomlaridan foydalanib, deklarativ JSON so'rov yuboradi. SQL haqida bilim shart emas.

### 1. So'rov Strukturasi

Root kalit quyidagi uchta variantdan biri:

| Kalit     | Vazifasi                                       |
|-----------|------------------------------------------------|
| `@data`   | Bitta obyekt `{...}` qaytaradi                 |
| `@data[]` | Massiv `[{...}, {...}]` qaytaradi              |
| `@info`   | Jadval va relation strukturasi haqida ma'lumot |

**Minimal namuna:**
```json
{
  "@data[]": {
    "@source": "emp[status: 1, $limit: 20]",
    "@fields": ["id", "full_name"]
  }
}
```

---

### 2. Direktivalar

| Direktiva  | Vazifasi                                                | Majburiy |
|------------|---------------------------------------------------------|----------|
| `@source`  | Manba jadval (alias), filtrlar va konfiguratsiya        | Ha       |
| `@fields`  | Qaytariladigan maydonlar va ularni nomlash              | Yo'q     |
| `@flatten` | Bola node maydonlarini ota nodega birlashtirib yuborish | Yo'q     |
| `[]`       | Kalit oxiriga qo'shilsa — massiv (array) qaytaradi      | Yo'q     |

#### `@source` Sintaksisi

```
alias[maydon: qiymat, maydon: operator qiymat, $limit: N, $offset: N, $order: ustun DIR, $join: tur, $rel: rel_nomi]
```

#### `@fields` Ikki Formatda

**Massiv (oddiy ismlash):**
```json
"@fields": ["id", "full_name", "status"]
```

**Obyekt (qo'shimcha nomlash va iboralar):**
```json
"@fields": {
  "employee_id":  "id",
  "ism_sharif":   "full_name",
  "tug_ilgan_kun": "TO_CHAR(TO_TIMESTAMP(birthday), 'DD.MM.YYYY')"
}
```

> `"*"` — barcha ruxsat etilgan maydonlarni olish:
> ```json
> "@fields": ["*"]
> ```

---

### 3. Filtr Operatorlari

`@source` ichida maydon filtrlari:

| Operator | Ma'nosi          | Misol                |
|----------|------------------|----------------------|
| `:`      | Teng (=)         | `status: 1`          |
| `!:`     | Teng emas (!=)   | `type: !: 0`         |
| `>`      | Katta (>)        | `age: > 18`          |
| `<`      | Kichik (<)       | `age: < 65`          |
| `..`     | Oraliq (BETWEEN) | `id: 1..100`         |
| `~`      | O'xshash (LIKE)  | `name: ~ Ali%`       |
| `in`     | Ro'yxatda (IN)   | `rank: in (1, 2, 3)` |

**Misol:**
```json
"@source": "emp[status: 1, id: 100..500, full_name: ~ Aliyev%, $limit: 10]"
```

---

### 4. Konfiguratsiya Parametrlari

| Parametr  | Misol             | Izoh                            |
|-----------|-------------------|---------------------------------|
| `$limit`  | `$limit: 20`      | Qaytariladigan qatorlar soni    |
| `$offset` | `$offset: 40`     | Skip qilish (sahifalash uchun)  |
| `$order`  | `$order: id DESC` | Tartiblash (`ASC` yoki `DESC`)  |
| `$join`   | `$join: left`     | JOIN turini qo'lda o'zgartirish |
| `$rel`    | `$rel: emp_admin` | Aniq relation nomini ko'rsatish |

**`$join` qiymatlari:**

| Qiymat              | JOIN turi  |
|---------------------|------------|
| `left` yoki `->`    | LEFT JOIN  |
| `right` yoki `<-`   | RIGHT JOIN |
| `inner` yoki `-><-` | INNER JOIN |
| `full` yoki `<->`   | FULL JOIN  |

---

### 5. Virtual Maydonlar Bilan Ishlash

Backend whitelist'da virtual maydon (SQL expression) aniqlagan bo'lsa, frontend uni oddiy jadval ustuni kabi ishlatadi — hech qanday farq yo'q:

**Backend whitelist:**
```json
{
  "shtat_department_basic:departmentBasic": {
    "id":           "id",
    "name":         "name_uz",
    "status":       "status",
    "has_children": "EXISTS(SELECT 1 FROM shtat_department_basic WHERE parent_id = departmentBasic.id)",
    "is_current":   "CASE WHEN current_position = 1 THEN true ELSE false END",
    "start_time":   "TO_CHAR(TO_TIMESTAMP(start_date), 'DD.MM.YYYY')"
  }
}
```

**Frontend so'rovi (oddiy ustundek ishlatadi):**
```json
{
  "@data[]": {
    "@source": "departmentBasic[status: 1, has_children: true]",
    "@fields": {
      "id":           "id",
      "name":         "name",
      "has_children": "has_children",
      "start_time":   "start_time"
    }
  }
}
```

> ✅ `has_children: true` — filtr sifatida ham ishlaydi. Engine qiymat `boolean` tipda ekanligini payqab, SQL'da `EXISTS(...) = true` ga o'giradi.

---

### 6. So'rov Namunalari

#### Namuna 1: Oddiy Ro'yxat (Paginatsiya)

```json
{
  "@data[]": {
    "@source": "emp[status: 1, $limit: 20, $offset: 0, $order: id DESC]",
    "@fields": ["id", "full_name", "jshshir", "birthDay"]
  }
}
```

#### Namuna 2: Bitta Obyekt

```json
{
  "@data": {
    "@source": "emp[status: 1, id: 42]",
    "@fields": {
      "id":        "id",
      "full_name": "full_name",
      "birthDay":  "birthDay",
      "jshshir":   "jshshir"
    }
  }
}
```

#### Namuna 3: Ichma-ich Massiv (One-to-Many)

```json
{
  "@data": {
    "@source": "emp[status: 1, id: 42]",
    "@fields": { "id": "id", "full_name": "full_name" },

    "positions[]": {
      "@source": "departmentStaffPosition[status: 1, is_current: true, $limit: 5]",
      "@fields": {
        "id":         "id",
        "begin_date": "start_time"
      }
    },

    "educations[]": {
      "@source": "education[$limit: 10, $order: id DESC]",
      "@fields": {
        "id":           "id",
        "diploma_type": "diploma_type_name"
      }
    }
  }
}
```

#### Namuna 4: @flatten bilan Daraxt Yasash

`@flatten: true` — bola node maydonlarini ota nodega birlashtirib beradi, alohida ichki obyekt yaratilmaydi:

```
Flattensiz:  { "degree": { "id": 5, "info": { "name": "Kapitan" } } }
Flattenli:   { "degree": { "id": 5, "name": "Kapitan" } }
```

```json
{
  "@data": {
    "@source": "emp[status: 1, id: 1..100, $limit: 2, $order: id DESC]",
    "@fields": { "id": "id", "full_name": "full_name", "birthDay": "birthDay" },

    "boshqarma": {
      "@source": "org[status: 1]",
      "@flatten": true,
      "@fields": { "viloyat_name": "title" }
    },

    "positions[]": {
      "@source": "departmentStaffPosition[is_current: true, $limit: 5]",
      "@fields": { "id": "id", "begin_date": "start_time" },

      "staffPosition": {
        "@source": "staffPositionBasic[status: 1]",
        "@flatten": true,
        "@fields": { "position_name": "name_uz" }
      }
    },

    "degree": {
      "@source": "militaryDegree[current_degree: 1]",
      "@flatten": true,
      "@fields": { "degree_name": "name_uz", "degree_date": "degree_given_time" }
    }
  }
}
```

**Natija strukturasi:**
```json
{
  "id": 42,
  "full_name": "Majidov Botir",
  "birthDay": "01.01.1993",
  "viloyat_name": "Jizzax viloyat",
  "positions": [
    { "id": 105, "begin_date": "01.06.2023", "position_name": "Buxgalter" },
    { "id": 78,  "begin_date": "15.01.2021", "position_name": "Yordamchi" }
  ],
  "degree_name": "Kapitan",
  "degree_date": "20.09.2022"
}
```

#### Namuna 5: Virtual Maydon Bilan Filtr

```json
{
  "@data[]": {
    "@source": "emp[status: 1, id: 21480..66580, $limit: 10]",
    "@fields": { "id": "id", "jshshir": "jshshir", "full_name": "full_name" },

    "positions": {
      "@source": "departmentStaffPosition[status: 1, is_current: true, has_children: true]",
      "@fields": { "id": "id", "begin_date": "start_time" },

      "ishJoyi": {
        "@source": "departmentBasic[status: 1, id: 15202]",
        "@fields": ["*"]
      }
    },

    "educations[]": {
      "@source": "education[$limit: 10, $order: id DESC]",
      "@fields": { "id": "id", "diploma_type": "diploma_type_name" }
    }
  }
}
```

#### Namuna 6: Tizim Strukturasini O'qish

```json
{ "@info": ["@tables", "@relations"] }
```

---

## Chiqish Formati (Output)

### Muvaffaqiyatli Natija

```json
{
  "isOk": true,
  "sql": "SELECT COALESCE(json_agg(t.uaq_data), '[]'::json) FROM (...) t",
  "params": {
    "p1": "1",
    "p2": "42"
  },
  "message": "success"
}
```

### Xatolik Natijasi

```json
{
  "isOk": false,
  "sql": null,
  "params": null,
  "message": "Generation Error: Column 'password' does not exist in table 'emp'"
}
```

### `@info` Natijasi

```json
{
  "isOk": true,
  "sql": "WITH input_json AS (...) SELECT jsonb_build_object('tables', ...) AS result;",
  "message": "info",
  "relations": ["emp->org", "emp->dept"]
}
```

SQL ni PostgreSQL ga yuborib:
```json
{
  "tables": {
    "emp": {
      "id":        "integer",
      "full_name": "character varying",
      "status":    "integer",
      "birthDay":  "character varying"
    },
    "departmentBasic": {
      "id":           "integer",
      "name":         "character varying",
      "status":       "integer",
      "has_children": "boolean"
    }
  },
  "relations": ["emp->org", "emp->dept", "dept->deptBasic"]
}
```

---

## Xavfsizlik

UAQ Engine ko'p qatlamli xavfsizlik tizimiga ega:

| Qatlam                          | Tavsif                                                                                                   |
|---------------------------------|----------------------------------------------------------------------------------------------------------|
| **Parametrlash**                | Barcha filtr qiymatlari `:p1`, `:p2` shaklida — SQL injection imkonsiz                                   |
| **Whitelist**                   | Frontend faqat ruxsat etilgan jadval va ustunlarga murojaat qila oladi                                   |
| **Global Tahdid Detektori**     | `DROP`, `DELETE`, `--`, `/* */` kabi xavfli konstruktsiyalar qat'iy bloklanadi                           |
| **Funksiya Ro'yxati**           | Faqat ruxsat etilgan SQL funksiyalar: `CONCAT`, `TO_CHAR`, `COALESCE`, `CASE WHEN`, `EXISTS` va hokazo   |
| **Identifikator Validatsiyasi** | Jadval va ustun nomlari `[a-zA-Z0-9_]` dan tashqari belgilarni qabul qilmaydi                            |
| **Alias Majburiyati**           | Whitelist'da alias berilgan jadvalga frontend to'g'ridan-to'g'ri haqiqiy nom bilan murojaat qila olmaydi |

---

## Qo'shimcha Imkoniyatlar

### Fayl Ma'lumotlarini Base64 ga Aylantirish

Bazadan kelgan JSON ichidagi fayl yo'llarini Base64 ga o'girish:

```c
char* uaq_inject_base64_files(
    const char* json_result,     // Bazadan kelgan JSON string
    const char* root_files_path, // "/var/www/my-project"
    const char* trigger_prefix   // "/web/uploads/"
);
```

**PHP da:**
```php
$raw   = $ffi->uaq_inject_base64_files($dbJsonString, '/var/www/project', '/uploads/');
$final = json_decode(\FFI::string($raw), true);
$ffi->uaq_free_string($raw);
```

**Natija:**
```
"/uploads/pasport.pdf" → "data:application/pdf;base64,JVBERi0x..."
```

Qo'llab-quvvatlanadigan MIME turlar: `jpg/jpeg`, `png`, `gif`, `webp`, `svg`, `pdf`, `mp4` va boshqalar.

> **Qachon ishlatish kerak:** Loyihalar arasi bir martali ma'lumot uzatishda.
> **Qachon ishlatmaslik kerak:** Umumiy ro'yxat endpointlarida — Base64 hajmni ~33% kattalashtiradi.

---

## Ko'p Tilli Integratsiya

| Platforma | Kutubxona formati | Usul                 |
|-----------|-------------------|----------------------|
| PHP       | `.so`             | FFI                  |
| Python    | `.so`             | `ctypes` / `cffi`    |
| Node.js   | `.wasm`           | WebAssembly          |
| Java      | `.so`             | JNI / Project Panama |
| Go        | `.so`             | `cgo`                |

**C-Header:**
```c
char* uaq_parse(
    const char* json_input,
    const char* whitelist,
    const char* relations,
    const char* macros_input
);

char* uaq_inject_base64_files(
    const char* json_result,
    const char* root_files_path,
    const char* trigger_prefix
);

void uaq_free_string(char* s);
```

---

## Xulosa

UAQ Engine backend va frontend o'rtasidagi og'ir yuk — SQL yozish, JOIN qurish, xavfsizlikni ta'minlash — ni to'liq o'z zimmasiga oladi:

- **Backend** faqat whitelist, relations va macros yozadi (bir marta)
- **Frontend** deklarativ JSON yuboradi (har safar)
- **UAQ** ikkalasini ko'priklab, xavfsiz va optimal SQL generatsiya qiladi
