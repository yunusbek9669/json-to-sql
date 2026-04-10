import ctypes
import json
import gc

lib = ctypes.CDLL("./target/release/libjson_to_sql.so")

lib.uaq_parse.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_char_p]
lib.uaq_parse.restype = ctypes.POINTER(ctypes.c_char)

lib.uaq_free_string.argtypes = [ctypes.POINTER(ctypes.c_char)]

json_input = b"""{
"@data": {
"@source": "emp[status: 1, id: 1000..2145, $limit: 20, $order: id DESC]",
"@fields": {
"id": "id",
"full_name": "full_name",
"passport": "jshshir",
"tug\xe2\x80\x98ilgan sana": "birthDay"
},
"position": {
"@source": "departmentStaffPosition[is_current: true]",
"@fields": {
"id": "id",
"begin_date": "TO_CHAR(TO_TIMESTAMP(start_time), 'DD.MM.YYYY')"
},
"optional ky name": {
"0": {
"@source": "org[status: 1]",
"@flatten": true,
"@fields": {
"viloyat boshqarma": "name"
}
},
"1": {
"@source": "innerOrg[status: 1]",
"@flatten": true,
"@fields": {
"tuman boshqarma": "name_uz"
}
},
"2": {
"@source": "departmentBasic[status: 1]",
"@flatten": true,
"@fields": {
"bo\xe2\x80\x98lim": "name_uz"
}
},
"3": {
"@source": "staffPositionBasic[status: 1]",
"@flatten": true,
"0": {
"@source": "staffPosition[status: 1]",
"@flatten": true,
"@fields": {
"name": "name_uz"
}
}
}
}
}
}
}"""

wl = b"""{
    "emp": ["*"], "departmentStaffPosition": ["*"], "org": ["*"],
    "innerOrg": ["*"], "departmentBasic": ["*"], "staffPositionBasic": ["*"],
    "staffPosition": ["*"]
}"""

rels = b"""{
    "emp->departmentStaffPosition": "LEFT JOIN @table ON @1.a=@2.b",
    "departmentStaffPosition<->org": "LEFT JOIN @table ON @1.c=@2.d",
    "departmentStaffPosition<->innerOrg": "LEFT JOIN @table ON @1.c=@2.d",
    "departmentStaffPosition<->departmentBasic": "LEFT JOIN @table ON @1.c=@2.d",
    "departmentStaffPosition<->staffPositionBasic": "LEFT JOIN @table ON @1.c=@2.d",
    "staffPositionBasic<->staffPosition": "LEFT JOIN @table ON @1.c=@2.d"
}"""

result_ptr = lib.uaq_parse(json_input, wl, rels)
result_bytes = ctypes.cast(result_ptr, ctypes.c_char_p).value
print("Result from FFI:", result_bytes.decode('utf-8'))
lib.uaq_free_string(result_ptr)

