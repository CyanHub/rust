//! Various number casting functions

use crate::prelude::*;

pub(crate) fn clif_intcast(
    fx: &mut FunctionCx<'_, '_, '_>,
    val: Value,
    to: Type,
    signed: bool,
) -> Value {
    let from = fx.bcx.func.dfg.value_type(val);
    match (from, to) {
        // equal
        (_, _) if from == to => val,

        // extend
        (_, _) if to.wider_or_equal(from) => {
            if signed {
                fx.bcx.ins().sextend(to, val)
            } else {
                fx.bcx.ins().uextend(to, val)
            }
        }

        // reduce
        (_, _) => fx.bcx.ins().ireduce(to, val),
    }
}

pub(crate) fn clif_int_or_float_cast(
    fx: &mut FunctionCx<'_, '_, '_>,
    from: Value,
    from_signed: bool,
    to_ty: Type,
    to_signed: bool,
) -> Value {
    let from_ty = fx.bcx.func.dfg.value_type(from);

    if from_ty.is_int() && to_ty.is_int() {
        // int-like -> int-like
        clif_intcast(
            fx,
            from,
            to_ty,
            // This is correct as either from_signed == to_signed (=> this is trivially correct)
            // Or from_clif_ty == to_clif_ty, which means this is a no-op.
            from_signed,
        )
    } else if from_ty.is_int() && to_ty.is_float() {
        if from_ty == types::I128 {
            // _______ss__f_
            // __float  tisf: i128 -> f32
            // __float  tidf: i128 -> f64
            // __floatuntisf: u128 -> f32
            // __floatuntidf: u128 -> f64

            let name = format!(
                "__float{sign}ti{flt}f",
                sign = if from_signed { "" } else { "un" },
                flt = match to_ty {
                    types::F32 => "s",
                    types::F64 => "d",
                    _ => unreachable!("{:?}", to_ty),
                },
            );

            let from_rust_ty = if from_signed { fx.tcx.types.i128 } else { fx.tcx.types.u128 };

            let to_rust_ty = match to_ty {
                types::F32 => fx.tcx.types.f32,
                types::F64 => fx.tcx.types.f64,
                _ => unreachable!(),
            };

            return fx
                .easy_call(&name, &[CValue::by_val(from, fx.layout_of(from_rust_ty))], to_rust_ty)
                .load_scalar(fx);
        }

        // int-like -> float
        if from_signed {
            fx.bcx.ins().fcvt_from_sint(to_ty, from)
        } else {
            fx.bcx.ins().fcvt_from_uint(to_ty, from)
        }
    } else if from_ty.is_float() && to_ty.is_int() {
        let val = if to_ty == types::I128 {
            // _____sssf___
            // __fix   sfti: f32 -> i128
            // __fix   dfti: f64 -> i128
            // __fixunssfti: f32 -> u128
            // __fixunsdfti: f64 -> u128

            let name = format!(
                "__fix{sign}{flt}fti",
                sign = if to_signed { "" } else { "uns" },
                flt = match from_ty {
                    types::F32 => "s",
                    types::F64 => "d",
                    _ => unreachable!("{:?}", to_ty),
                },
            );

            let from_rust_ty = match from_ty {
                types::F32 => fx.tcx.types.f32,
                types::F64 => fx.tcx.types.f64,
                _ => unreachable!(),
            };

            let to_rust_ty = if to_signed { fx.tcx.types.i128 } else { fx.tcx.types.u128 };

            fx.easy_call(&name, &[CValue::by_val(from, fx.layout_of(from_rust_ty))], to_rust_ty)
                .load_scalar(fx)
        } else if to_ty == types::I8 || to_ty == types::I16 {
            // FIXME implement fcvt_to_*int_sat.i8/i16
            let val = if to_signed {
                fx.bcx.ins().fcvt_to_sint_sat(types::I32, from)
            } else {
                fx.bcx.ins().fcvt_to_uint_sat(types::I32, from)
            };
            let (min, max) = match (to_ty, to_signed) {
                (types::I8, false) => (0, i64::from(u8::MAX)),
                (types::I16, false) => (0, i64::from(u16::MAX)),
                (types::I8, true) => (i64::from(i8::MIN), i64::from(i8::MAX)),
                (types::I16, true) => (i64::from(i16::MIN), i64::from(i16::MAX)),
                _ => unreachable!(),
            };
            let min_val = fx.bcx.ins().iconst(types::I32, min);
            let max_val = fx.bcx.ins().iconst(types::I32, max);

            let val = if to_signed {
                let has_underflow = fx.bcx.ins().icmp_imm(IntCC::SignedLessThan, val, min);
                let has_overflow = fx.bcx.ins().icmp_imm(IntCC::SignedGreaterThan, val, max);
                let bottom_capped = fx.bcx.ins().select(has_underflow, min_val, val);
                fx.bcx.ins().select(has_overflow, max_val, bottom_capped)
            } else {
                let has_overflow = fx.bcx.ins().icmp_imm(IntCC::UnsignedGreaterThan, val, max);
                fx.bcx.ins().select(has_overflow, max_val, val)
            };
            fx.bcx.ins().ireduce(to_ty, val)
        } else if to_signed {
            fx.bcx.ins().fcvt_to_sint_sat(to_ty, from)
        } else {
            fx.bcx.ins().fcvt_to_uint_sat(to_ty, from)
        };

        if let Some(false) = fx.tcx.sess.opts.unstable_opts.saturating_float_casts {
            return val;
        }

        let is_not_nan = fx.bcx.ins().fcmp(FloatCC::Equal, from, from);
        let zero = fx.bcx.ins().iconst(to_ty, 0);
        fx.bcx.ins().select(is_not_nan, val, zero)
    } else if from_ty.is_float() && to_ty.is_float() {
        // float -> float
        match (from_ty, to_ty) {
            (types::F32, types::F64) => fx.bcx.ins().fpromote(types::F64, from),
            (types::F64, types::F32) => fx.bcx.ins().fdemote(types::F32, from),
            _ => from,
        }
    } else {
        unreachable!("cast value from {:?} to {:?}", from_ty, to_ty);
    }
}
