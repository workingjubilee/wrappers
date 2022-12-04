use crate::prelude::*;
use pgx::{is_a, pg_sys, pg_sys::Datum, FromDatum, PgBuiltInOids, PgList, PgOid};
use std::ffi::CStr;

// create array of Cell from constant datum array
pub(crate) unsafe fn form_array_from_datum(
    datum: Datum,
    is_null: bool,
    typoid: pg_sys::Oid,
) -> Option<Vec<Cell>> {
    if is_null {
        return None;
    }

    let oid = PgOid::from(typoid);
    match oid {
        PgOid::BuiltIn(PgBuiltInOids::BOOLARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::BOOLOID)
        }
        PgOid::BuiltIn(PgBuiltInOids::CHARARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::CHAROID)
        }
        PgOid::BuiltIn(PgBuiltInOids::INT2ARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::INT2OID)
        }
        PgOid::BuiltIn(PgBuiltInOids::FLOAT4ARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::FLOAT4OID)
        }
        PgOid::BuiltIn(PgBuiltInOids::INT4ARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::INT4OID)
        }
        PgOid::BuiltIn(PgBuiltInOids::FLOAT8ARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::FLOAT8OID)
        }
        PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::INT8OID)
        }
        PgOid::BuiltIn(PgBuiltInOids::TEXTARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::TEXTOID)
        }
        PgOid::BuiltIn(PgBuiltInOids::DATEARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::DATEOID)
        }
        PgOid::BuiltIn(PgBuiltInOids::TIMESTAMPARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::TIMESTAMPOID)
        }
        PgOid::BuiltIn(PgBuiltInOids::JSONBARRAYOID) => {
            Vec::<Cell>::from_polymorphic_datum(datum, false, pg_sys::JSONBOID)
        }
        _ => None,
    }
}

pub(crate) unsafe fn get_operator(opno: pg_sys::Oid) -> pg_sys::Form_pg_operator {
    let htup = pg_sys::SearchSysCache1(
        pg_sys::SysCacheIdentifier_OPEROID.try_into().unwrap(),
        opno.try_into().unwrap(),
    );
    if htup.is_null() {
        pg_sys::ReleaseSysCache(htup);
        pgx::error!("cache lookup operator {} failed", opno);
    }
    let op = pg_sys::pgx_GETSTRUCT(htup) as pg_sys::Form_pg_operator;
    pg_sys::ReleaseSysCache(htup);
    op
}

pub(crate) unsafe fn unnest_clause(node: *mut pg_sys::Node) -> *mut pg_sys::Node {
    if is_a(node, pg_sys::NodeTag_T_RelabelType) {
        (*(node as *mut pg_sys::RelabelType)).arg as _
    } else if is_a(node, pg_sys::NodeTag_T_ArrayCoerceExpr) {
        (*(node as *mut pg_sys::ArrayCoerceExpr)).arg as _
    } else {
        node
    }
}

pub(crate) unsafe fn extract_from_op_expr(
    _root: *mut pg_sys::PlannerInfo,
    baserel_id: pg_sys::Oid,
    baserel_ids: pg_sys::Relids,
    expr: *mut pg_sys::OpExpr,
) -> Option<Qual> {
    let args: PgList<pg_sys::Node> = PgList::from_pg((*expr).args);

    // only deal with binary operator
    if args.len() != 2 {
        return None;
    }

    // get operator
    let opno = (*expr).opno;
    let opr = get_operator(opno);
    if opr.is_null() {
        return None;
    }

    let mut left = unnest_clause(args.head().unwrap());
    let mut right = unnest_clause(args.tail().unwrap());

    // swap operands if needed
    if is_a(right, pg_sys::NodeTag_T_Var)
        && !is_a(left, pg_sys::NodeTag_T_Var)
        && (*opr).oprcom != 0
    {
        std::mem::swap(&mut left, &mut right);
    }

    if is_a(left, pg_sys::NodeTag_T_Var) && is_a(right, pg_sys::NodeTag_T_Const) {
        let left = left as *mut pg_sys::Var;
        let right = right as *mut pg_sys::Const;

        if pg_sys::bms_is_member((*left).varno.try_into().unwrap(), baserel_ids)
            && (*left).varattno >= 1
        {
            let field = pg_sys::get_attname(baserel_id, (*left).varattno, false);
            let value = Cell::from_polymorphic_datum(
                (*right).constvalue,
                (*right).constisnull,
                (*right).consttype,
            );
            if let Some(value) = value {
                let qual = Qual {
                    field: CStr::from_ptr(field).to_str().unwrap().to_string(),
                    operator: pgx::name_data_to_str(&(*opr).oprname).to_string(),
                    value: Value::Cell(value),
                    use_or: false,
                };
                return Some(qual);
            }
        }
    }

    None
}

pub(crate) unsafe fn extract_from_null_test(
    baserel_id: pg_sys::Oid,
    expr: *mut pg_sys::NullTest,
) -> Option<Qual> {
    let var = (*expr).arg as *mut pg_sys::Var;
    if !is_a(var as _, pg_sys::NodeTag_T_Var) || (*var).varattno < 1 {
        return None;
    }

    let field = pg_sys::get_attname(baserel_id, (*var).varattno, false);

    let opname = if (*expr).nulltesttype == pg_sys::NullTestType_IS_NULL {
        "is".to_string()
    } else {
        "is not".to_string()
    };

    let qual = Qual {
        field: CStr::from_ptr(field).to_str().unwrap().to_string(),
        operator: opname,
        value: Value::Cell(Cell::String("null".to_string())),
        use_or: false,
    };

    Some(qual)
}

pub(crate) unsafe fn extract_from_scalar_array_op_expr(
    _root: *mut pg_sys::PlannerInfo,
    baserel_id: pg_sys::Oid,
    baserel_ids: pg_sys::Relids,
    expr: *mut pg_sys::ScalarArrayOpExpr,
) -> Option<Qual> {
    let args: PgList<pg_sys::Node> = PgList::from_pg((*expr).args);

    // only deal with binary operator
    if args.len() != 2 {
        return None;
    }

    // get operator
    let opno = (*expr).opno;
    let opr = get_operator(opno);
    if opr.is_null() {
        return None;
    }

    let left = unnest_clause(args.head().unwrap());
    let right = unnest_clause(args.tail().unwrap());

    if is_a(left, pg_sys::NodeTag_T_Var) && is_a(right, pg_sys::NodeTag_T_Const) {
        let left = left as *mut pg_sys::Var;
        let right = right as *mut pg_sys::Const;

        if pg_sys::bms_is_member((*left).varno.try_into().unwrap(), baserel_ids)
            && (*left).varattno >= 1
        {
            let field = pg_sys::get_attname(baserel_id, (*left).varattno, false);

            let value: Option<Vec<Cell>> = form_array_from_datum(
                (*right).constvalue,
                (*right).constisnull,
                (*right).consttype,
            );
            if let Some(value) = value {
                let qual = Qual {
                    field: CStr::from_ptr(field).to_str().unwrap().to_string(),
                    operator: pgx::name_data_to_str(&(*opr).oprname).to_string(),
                    value: Value::Array(value),
                    use_or: (*expr).useOr,
                };
                return Some(qual);
            }
        }
    }

    None
}

pub(crate) unsafe fn extract_quals(
    root: *mut pg_sys::PlannerInfo,
    baserel: *mut pg_sys::RelOptInfo,
    baserel_id: pg_sys::Oid,
) -> Vec<Qual> {
    let mut quals = Vec::new();

    let conds = PgList::<pg_sys::RestrictInfo>::from_pg((*baserel).baserestrictinfo);
    for cond in conds.iter_ptr() {
        let expr = (*cond).clause as *mut pg_sys::Node;
        let extracted = if is_a(expr, pg_sys::NodeTag_T_OpExpr) {
            extract_from_op_expr(root, baserel_id, (*baserel).relids, expr as _)
        } else if is_a(expr, pg_sys::NodeTag_T_NullTest) {
            extract_from_null_test(baserel_id, expr as _)
        } else if is_a(expr, pg_sys::NodeTag_T_ScalarArrayOpExpr) {
            extract_from_scalar_array_op_expr(root, baserel_id, (*baserel).relids, expr as _)
        } else {
            if let Some(stm) = pgx::nodes::node_to_string(expr) {
                report_warning(&format!("unsupported qual: {}", stm));
            }
            None
        };

        if let Some(qual) = extracted {
            quals.push(qual);
        }
    }

    quals
}
