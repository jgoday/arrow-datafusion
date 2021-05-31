// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

//! Remove duplicate filters optimizer rule

use crate::execution::context::ExecutionProps;
use crate::logical_plan::LogicalPlan;
use crate::logical_plan::{lit, Expr};
use crate::optimizer::optimizer::OptimizerRule;
use crate::optimizer::utils;
use crate::optimizer::utils::optimize_explain;
use crate::scalar::ScalarValue;
use crate::{error::Result, logical_plan::Operator};

/// Remove duplicate filters optimizer.
/// # Introduction
/// It uses boolean algebra laws to simplify or reduce the number of terms in expressions.
///
/// Filter: #b Gt Int32(2) And #b Gt Int32(2)
/// is optimized to
/// Filter: #b Gt Int32(2)
pub struct RemoveDuplicateFilters {}

fn expr_contains<'a>(expr: &'a Expr, needle: &'a Expr) -> bool {
    match expr {
        Expr::BinaryExpr {
            left,
            op: Operator::And,
            right,
        } => expr_contains(left, needle) || expr_contains(right, needle),
        Expr::BinaryExpr {
            left,
            op: Operator::Or,
            right,
        } => expr_contains(left, needle) || expr_contains(right, needle),
        _ => expr == needle,
    }
}

fn as_binary_expr<'a>(expr: &'a Expr) -> Option<&'a Expr> {
    match expr {
        Expr::BinaryExpr { .. } => Some(expr),
        _ => None,
    }
}

fn operator_is_boolean(op: &Operator) -> bool {
    op == &Operator::And || op == &Operator::Or
}

fn is_one<'a>(s: &'a Expr) -> bool {
    match s {
        Expr::Literal(ScalarValue::Int8(Some(1))) => true,
        Expr::Literal(ScalarValue::Int16(Some(1))) => true,
        Expr::Literal(ScalarValue::Int32(Some(1))) => true,
        Expr::Literal(ScalarValue::Int64(Some(1))) => true,
        Expr::Literal(ScalarValue::UInt8(Some(1))) => true,
        Expr::Literal(ScalarValue::UInt16(Some(1))) => true,
        Expr::Literal(ScalarValue::UInt32(Some(1))) => true,
        Expr::Literal(ScalarValue::UInt64(Some(1))) => true,
        Expr::Literal(ScalarValue::Float32(Some(v))) if *v == 1. => true,
        Expr::Literal(ScalarValue::Float64(Some(v))) if *v == 1. => true,
        _ => false
    }
}

fn is_zero<'a>(s: &'a Expr) -> bool {
    match s {
        Expr::Literal(ScalarValue::Int8(Some(0))) => true,
        Expr::Literal(ScalarValue::Int16(Some(0))) => true,
        Expr::Literal(ScalarValue::Int32(Some(0))) => true,
        Expr::Literal(ScalarValue::Int64(Some(0))) => true,
        Expr::Literal(ScalarValue::UInt8(Some(0))) => true,
        Expr::Literal(ScalarValue::UInt16(Some(0))) => true,
        Expr::Literal(ScalarValue::UInt32(Some(0))) => true,
        Expr::Literal(ScalarValue::UInt64(Some(0))) => true,
        Expr::Literal(ScalarValue::Float32(Some(v))) if *v == 0. => true,
        Expr::Literal(ScalarValue::Float64(Some(v))) if *v == 0. => true,
        _ => false
    }
}

fn is_true<'a>(expr: &'a Expr) -> bool {
    match expr {
        Expr::Literal(ScalarValue::Boolean(Some(v))) => *v,
        _ => false,
    }
}

fn is_false<'a>(expr: &'a Expr) -> bool {
    match expr {
        Expr::Literal(ScalarValue::Boolean(Some(v))) => *v == false,
        _ => false,
    }
}

fn simplify<'a>(expr: &'a Expr) -> Expr {
    match expr {
        Expr::BinaryExpr {
            left,
            op: Operator::Or,
            right,
        } if is_true(left) || is_true(right) => lit(true),
        Expr::BinaryExpr {
            left,
            op: Operator::Or,
            right,
        } if is_false(left) => simplify(right),
        Expr::BinaryExpr {
            left,
            op: Operator::Or,
            right,
        } if is_false(right) => simplify(left),
        Expr::BinaryExpr {
            left,
            op: Operator::Or,
            right,
        } if left == right => simplify(left),
        Expr::BinaryExpr {
            left,
            op: Operator::And,
            right,
        } if is_false(left) || is_false(right) => lit(false),
        Expr::BinaryExpr {
            left,
            op: Operator::And,
            right,
        } if is_true(right) => simplify(left),
        Expr::BinaryExpr {
            left,
            op: Operator::And,
            right,
        } if is_true(left) => simplify(right),
        Expr::BinaryExpr {
            left,
            op: Operator::And,
            right,
        } if left == right => simplify(right),
        Expr::BinaryExpr {
            left,
            op: Operator::Minus,
            right
        } if is_zero(left) => Expr::Negative(Box::new(simplify(right))),
        Expr::BinaryExpr {
            left,
            op: Operator::Minus,
            right
        } if is_zero(right) => simplify(left),
        Expr::BinaryExpr {
            left,
            op: Operator::Minus,
            right
        } if left == right => lit(0),
        Expr::BinaryExpr {
            left,
            op: Operator::Multiply,
            right
        } if is_zero(left) || is_zero(right) => lit(0),
        Expr::BinaryExpr {
            left,
            op: Operator::Multiply,
            right
        } if is_one(left) => simplify(right),
        Expr::BinaryExpr {
            left,
            op: Operator::Multiply,
            right
        } if is_one(right) => simplify(left),
        Expr::BinaryExpr {
            left,
            op: Operator::Divide,
            right
        } if is_one(right) => simplify(left),
        Expr::BinaryExpr {
            left,
            op: Operator::Divide,
            right
        } if left == right => lit(1),
        Expr::BinaryExpr {
            left,
            op: Operator::Plus,
            right
        } if is_zero(left) => simplify(right),
        Expr::BinaryExpr {
            left,
            op: Operator::Plus,
            right
        } if is_zero(right) => simplify(left),
        Expr::BinaryExpr { left, op, right }
            if left == right && operator_is_boolean(op) =>
        {
            simplify(left)
        }
        Expr::BinaryExpr {
            left,
            op: Operator::Or,
            right,
        } if expr_contains(left, right) => as_binary_expr(left)
            .map(|x| match x {
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::Or,
                    right: _,
                } => x.clone(),
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::And,
                    right: _,
                } => *right.clone(),
                _ => expr.clone(),
            })
            .unwrap_or(expr.clone()),
        Expr::BinaryExpr {
            left,
            op: Operator::Or,
            right,
        } if expr_contains(right, left) => as_binary_expr(right)
            .map(|x| match x {
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::Or,
                    right: _,
                } => *right.clone(),
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::And,
                    right: _,
                } => *left.clone(),
                _ => expr.clone(),
            })
            .unwrap_or(expr.clone()),
        Expr::BinaryExpr {
            left,
            op: Operator::And,
            right,
        } if expr_contains(left, right) => as_binary_expr(left)
            .map(|x| match x {
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::Or,
                    right: _,
                } => *right.clone(),
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::And,
                    right: _,
                } => x.clone(),
                _ => expr.clone(),
            })
            .unwrap_or(expr.clone()),
        Expr::BinaryExpr {
            left,
            op: Operator::And,
            right,
        } if expr_contains(right, left) => as_binary_expr(right)
            .map(|x| match x {
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::Or,
                    right: _,
                } => *left.clone(),
                Expr::BinaryExpr {
                    left: _,
                    op: Operator::And,
                    right: _,
                } => x.clone(),
                _ => expr.clone(),
            })
            .unwrap_or(expr.clone()),
        Expr::BinaryExpr { left, op, right } => Expr::BinaryExpr {
            left: Box::new(simplify(&left)),
            op: *op,
            right: Box::new(simplify(right)),
        },
        _ => expr.clone(),
    }
}

fn optimize(plan: &LogicalPlan) -> Result<LogicalPlan> {
    match plan {
        LogicalPlan::Filter { input, predicate } => Ok(LogicalPlan::Filter {
            input: input.clone(),
            predicate: simplify(predicate),
        }),
        LogicalPlan::Projection { expr, input, schema } => Ok(LogicalPlan::Projection {
            expr: expr.into_iter().map(|x| simplify(x)).collect::<Vec<_>>(),
            input: input.clone(),
            schema: schema.clone(),
        }),
        _ => {
            let new_inputs = plan
                .inputs()
                .iter()
                .map(|input| optimize(input))
                .collect::<Result<Vec<_>>>()?;

            let expr = plan.expressions();
            utils::from_plan(&plan, &expr, &new_inputs)
        }
    }
}

impl OptimizerRule for RemoveDuplicateFilters {
    fn name(&self) -> &str {
        "remove_duplicate_filters"
    }

    fn optimize(
        &self,
        plan: &LogicalPlan,
        execution_props: &ExecutionProps,
    ) -> Result<LogicalPlan> {
        match plan {
            LogicalPlan::Explain {
                verbose,
                plan,
                stringified_plans,
                schema,
            } => {
                let schema = schema.as_ref().to_owned().into();
                optimize_explain(
                    self,
                    *verbose,
                    &*plan,
                    stringified_plans,
                    &schema,
                    execution_props,
                )
            }
            _ => optimize(plan),
        }
    }
}

impl RemoveDuplicateFilters {
    #[allow(missing_docs)]
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logical_plan::{and, binary_expr, col, lit, Expr, LogicalPlanBuilder};
    use crate::test::*;

    fn assert_optimized_plan_eq(plan: &LogicalPlan, expected: &str) {
        let rule = RemoveDuplicateFilters::new();
        let optimized_plan = rule
            .optimize(plan, &ExecutionProps::new())
            .expect("failed to optimize plan");
        let formatted_plan = format!("{:?}", optimized_plan);
        assert_eq!(formatted_plan, expected);
    }

    #[test]
    fn test_simplify_or_true() -> Result<()> {
        let expr_a = binary_expr(col("c"), Operator::Or, lit(true));
        let expr_b = binary_expr(lit(true), Operator::Or, col("c"));
        let expected = lit(true);

        assert_eq!(simplify(&expr_a), expected);
        assert_eq!(simplify(&expr_b), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_or_false() -> Result<()> {
        let expr_a = binary_expr(lit(false), Operator::Or, col("c"));
        let expr_b = binary_expr(col("c"), Operator::Or, lit(false));
        let expected = col("c");

        assert_eq!(simplify(&expr_a), expected);
        assert_eq!(simplify(&expr_b), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_or_same() -> Result<()> {
        let expr = binary_expr(col("c"), Operator::Or, col("c"));
        let expected = col("c");

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_and_false() -> Result<()> {
        let expr_a = binary_expr(lit(false), Operator::And, col("c"));
        let expr_b = binary_expr(col("c"), Operator::And, lit(false));
        let expected = lit(false);

        assert_eq!(simplify(&expr_a), expected);
        assert_eq!(simplify(&expr_b), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_and_same() -> Result<()> {
        let expr = binary_expr(col("c"), Operator::And, col("c"));
        let expected = col("c");

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_and_true() -> Result<()> {
        let expr_a = binary_expr(lit(true), Operator::And, col("c"));
        let expr_b = binary_expr(col("c"), Operator::And, lit(true));
        let expected = col("c");

        assert_eq!(simplify(&expr_a), expected);
        assert_eq!(simplify(&expr_b), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_minus_zero() -> Result<()> {
        let expr = binary_expr(lit(0), Operator::Minus, col("c"));
        let expected = Expr::Negative(Box::new(col("c")));

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_minus_same() -> Result<()> {
        let expr = binary_expr(col("c"), Operator::Minus, col("c"));
        let expected = lit(0);

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_multiply_by_zero() -> Result<()> {
        let expr_a = binary_expr(col("c"), Operator::Multiply, lit(0));
        let expr_b = binary_expr(lit(0), Operator::Multiply, col("c"));
        let expected = lit(0);

        assert_eq!(simplify(&expr_a), expected);
        assert_eq!(simplify(&expr_b), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_multiply_by_one() -> Result<()> {
        let expr_a = binary_expr(col("c"), Operator::Multiply, lit(1));
        let expr_b = binary_expr(lit(1), Operator::Multiply, col("c"));
        let expected = col("c");

        assert_eq!(simplify(&expr_a), expected);
        assert_eq!(simplify(&expr_b), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_divide_by_one() -> Result<()> {
        let expr = binary_expr(col("c"), Operator::Divide, lit(1));
        let expected = col("c");

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_divide_by_same() -> Result<()> {
        let expr = binary_expr(col("c"), Operator::Divide, col("c"));
        let expected = lit(1);

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_cancel_sub() -> Result<()> {
        let expr = binary_expr(col("c"), Operator::Minus, col("c"));
        let expected = lit(0);

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_plus_zero() -> Result<()> {
        let expr_a = binary_expr(col("c"), Operator::Plus, lit(0));
        let expr_b = binary_expr(lit(0), Operator::Plus, col("c"));
        let expected = col("c");

        assert_eq!(simplify(&expr_a), expected);
        assert_eq!(simplify(&expr_b), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_simple_and() -> Result<()> {
        // (c > 5) AND (c > 5)
        let expr = binary_expr(col("c").gt(lit(5)), Operator::And, col("c").gt(lit(5)));
        let expected = col("c").gt(lit(5));

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_composed_and() -> Result<()> {
        // ((c > 5) AND (d < 6)) AND (c > 5)
        let expr = binary_expr(
            binary_expr(col("c").gt(lit(5)), Operator::And, col("d").lt(lit(6))),
            Operator::And,
            col("c").gt(lit(5)),
        );
        let expected =
            binary_expr(col("c").gt(lit(5)), Operator::And, col("d").lt(lit(6)));

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_negated_and() -> Result<()> {
        // (c > 5) AND !(c > 5) -- can't remove
        let expr = binary_expr(
            col("c").gt(lit(5)),
            Operator::And,
            Expr::not(col("c").gt(lit(5))),
        );
        let expected = expr.clone();

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_simplify_or_and() -> Result<()> {
        // (c > 5) OR ((d < 6) AND (c > 5) -- can't remove
        let expr = binary_expr(
            col("c").gt(lit(5)),
            Operator::Or,
            binary_expr(col("d").lt(lit(6)), Operator::And, col("c").gt(lit(5))),
        );
        let expected = col("c").gt(lit(5));

        assert_eq!(simplify(&expr), expected);
        Ok(())
    }

    #[test]
    fn test_do_not_simplify_arithmetic_expr() -> Result<()> {
        let expr_plus = binary_expr(lit(1), Operator::Plus, lit(1));
        let expr_eq = binary_expr(lit(1), Operator::Eq, lit(1));

        assert_eq!(simplify(&expr_plus), expr_plus);
        assert_eq!(simplify(&expr_eq), expr_eq);

        Ok(())
    }

    #[test]
    fn test_optimized_plan() -> Result<()> {
        let table_scan = test_table_scan()?;
        let plan = LogicalPlanBuilder::from(&table_scan)
            .project(vec![col("a")])?
            .filter(and(col("b").gt(lit(1)), col("b").gt(lit(1))))?
            .build()?;

        assert_optimized_plan_eq(
            &plan,
            "\
	        Filter: #b Gt Int32(1)\
            \n  Projection: #a\
            \n    TableScan: test projection=None",
        );
        Ok(())
    }

    // ((c > 5) AND (d < 6)) AND (c > 5) --> (c > 5) AND (d < 6)
    #[test]
    fn test_optimized_plan_with_composed_and() -> Result<()> {
        let table_scan = test_table_scan()?;
        let plan = LogicalPlanBuilder::from(&table_scan)
            .project(vec![col("a")])?
            .filter(and(
                and(col("a").gt(lit(5)), col("b").lt(lit(6))),
                col("a").gt(lit(5)),
            ))?
            .build()?;

        assert_optimized_plan_eq(
            &plan,
            "\
            Filter: #a Gt Int32(5) And #b Lt Int32(6)\
            \n  Projection: #a\
	        \n    TableScan: test projection=None",
        );
        Ok(())
    }
}
