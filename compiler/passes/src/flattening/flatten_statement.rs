// Copyright (C) 2019-2022 Aleo Systems Inc.
// This file is part of the Leo library.

// The Leo library is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// The Leo library is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with the Leo library. If not, see <https://www.gnu.org/licenses/>.

use crate::Flattener;
use std::borrow::Borrow;

use leo_ast::{
    AssignStatement, BinaryExpression, BinaryOperation, Block, ConditionalStatement, ConsoleFunction, ConsoleStatement,
    DefinitionStatement, Expression, ExpressionReconstructor, FinalizeStatement, Identifier, IterationStatement, Node,
    ReturnStatement, Statement, StatementReconstructor, TupleExpression, Type, UnaryExpression, UnaryOperation,
};

impl StatementReconstructor for Flattener<'_> {
    /// Flattens an assign statement, if necessary.
    /// Marks variables as structs as necessary.
    /// Note that new statements are only produced if the right hand side is a ternary expression over structs.
    /// Otherwise, the statement is returned as is.
    fn reconstruct_assign(&mut self, assign: AssignStatement) -> (Statement, Self::AdditionalOutput) {
        // Flatten the rhs of the assignment.
        let (value, mut statements) = self.reconstruct_expression(assign.value);
        match (assign.place, value) {
            // If the lhs is an identifier and the rhs is a tuple, then add the tuple to `self.tuples`.
            (Expression::Identifier(identifier), Expression::Tuple(tuple)) => {
                self.tuples.insert(identifier.name, tuple);
                // Note that tuple assignments are removed from the AST.
                (Statement::dummy(Default::default()), statements)
            }
            // If the lhs is an identifier and the rhs is an identifier that is a tuple, then add it to `self.tuples`.
            (Expression::Identifier(lhs_identifier), Expression::Identifier(rhs_identifier))
                if self.tuples.contains_key(&rhs_identifier.name) =>
            {
                // Lookup the entry in `self.tuples` and add it for the lhs of the assignment.
                // Note that the `unwrap` is safe since the match arm checks that the entry exists.
                self.tuples.insert(
                    lhs_identifier.name,
                    self.tuples.get(&rhs_identifier.name).unwrap().clone(),
                );
                // Note that tuple assignments are removed from the AST.
                (Statement::dummy(Default::default()), statements)
            }
            // If the lhs is an identifier and the rhs is a function call that produces a tuple, then add it to `self.tuples`.
            (Expression::Identifier(lhs_identifier), Expression::Call(call)) => {
                // Retrieve the entry in the symbol table for the function call.
                // Note that this unwrap is safe since type checking ensures that the function exists.
                let function_name = match call.function.borrow() {
                    Expression::Identifier(rhs_identifier) => rhs_identifier.name,
                    _ => unreachable!("Parsing guarantees that `function` is an identifier."),
                };

                let function = self.symbol_table.borrow().functions.get(&function_name).unwrap();
                match &function.output_type {
                    // If the function returns a tuple, reconstruct the assignment and add an entry to `self.tuples`.
                    Type::Tuple(tuple) => {
                        // Create a new tuple expression with unique identifiers for each index of the lhs.
                        let tuple_expression = TupleExpression {
                            elements: (0..tuple.len())
                                .map(|i| {
                                    Expression::Identifier(Identifier::new(
                                        self.assigner.unique_symbol(lhs_identifier.name, format!("$index${i}$")),
                                    ))
                                })
                                .collect(),
                            span: Default::default(),
                        };
                        // Add the `tuple_expression` to `self.tuples`.
                        self.tuples.insert(lhs_identifier.name, tuple_expression.clone());
                        // Construct a new assignment statement with a tuple expression on the lhs.
                        (
                            Statement::Assign(Box::new(AssignStatement {
                                place: Expression::Tuple(tuple_expression),
                                value: Expression::Call(call),
                                span: Default::default(),
                            })),
                            statements,
                        )
                    }
                    // Otherwise, reconstruct the assignment as is.
                    _ => (
                        Statement::Assign(Box::new(AssignStatement {
                            place: Expression::Identifier(lhs_identifier),
                            value: Expression::Call(call),
                            span: Default::default(),
                        })),
                        statements,
                    ),
                }
            }
            (Expression::Identifier(identifier), expression) => {
                self.update_structs(&identifier, &expression);
                (
                    self.assigner.simple_assign_statement(identifier, expression),
                    statements,
                )
            }
            // If the lhs is a tuple and the rhs is a function call, then return the reconstructed statement.
            (Expression::Tuple(tuple), Expression::Call(call)) => (
                Statement::Assign(Box::new(AssignStatement {
                    place: Expression::Tuple(tuple),
                    value: Expression::Call(call),
                    span: Default::default(),
                })),
                statements,
            ),
            // If the lhs is a tuple and the rhs is a tuple, create a new assign statement for each tuple element.
            (Expression::Tuple(lhs_tuple), Expression::Tuple(rhs_tuple)) => {
                statements.extend(lhs_tuple.elements.into_iter().zip(rhs_tuple.elements.into_iter()).map(
                    |(lhs, rhs)| {
                        Statement::Assign(Box::new(AssignStatement {
                            place: lhs,
                            value: rhs,
                            span: Default::default(),
                        }))
                    },
                ));
                (Statement::dummy(Default::default()), statements)
            }
            // If the lhs is a tuple and the rhs is an identifier that is a tuple, create a new assign statement for each tuple element.
            (Expression::Tuple(lhs_tuple), Expression::Identifier(identifier))
                if self.tuples.contains_key(&identifier.name) =>
            {
                // Lookup the entry in `self.tuples`.
                // Note that the `unwrap` is safe since the match arm checks that the entry exists.
                let rhs_tuple = self.tuples.get(&identifier.name).unwrap();
                // Create a new assign statement for each tuple element.
                statements.extend(
                    lhs_tuple
                        .elements
                        .into_iter()
                        .zip(rhs_tuple.elements.iter())
                        .map(|(lhs, rhs)| {
                            Statement::Assign(Box::new(AssignStatement {
                                place: lhs,
                                value: rhs.clone(),
                                span: Default::default(),
                            }))
                        }),
                );
                (Statement::dummy(Default::default()), statements)
            }
            // If the lhs of an assignment is a tuple, then the rhs can be one of the following:
            //  - A function call that produces a tuple. (handled above)
            //  - A tuple. (handled above)
            //  - An identifier that is a tuple. (handled above)
            //  - A ternary expression that produces a tuple. (handled when the rhs is flattened above)
            (Expression::Tuple(_), _) => {
                unreachable!("`Type checking guarantees that the rhs of an assignment to a tuple is a tuple.`")
            }
            _ => unreachable!("`AssignStatement`s can only have `Identifier`s or `Tuple`s on the left hand side."),
        }
    }

    // TODO: Do we want to flatten nested blocks? They do not affect code generation but it would regularize the AST structure.
    /// Flattens the statements inside a basic block.
    /// The resulting block does not contain any conditional statements.
    fn reconstruct_block(&mut self, block: Block) -> (Block, Self::AdditionalOutput) {
        let mut statements = Vec::with_capacity(block.statements.len());

        // Flatten each statement, accumulating any new statements produced.
        for statement in block.statements {
            let (reconstructed_statement, additional_statements) = self.reconstruct_statement(statement);
            statements.extend(additional_statements);
            statements.push(reconstructed_statement);
        }

        (
            Block {
                span: block.span,
                statements,
            },
            Default::default(),
        )
    }

    /// Flatten a conditional statement into a list of statements.
    fn reconstruct_conditional(&mut self, conditional: ConditionalStatement) -> (Statement, Self::AdditionalOutput) {
        let mut statements = Vec::with_capacity(conditional.then.statements.len());

        // Add condition to the condition stack.
        self.condition_stack.push(conditional.condition.clone());

        // Reconstruct the then-block and accumulate it constituent statements.
        statements.extend(self.reconstruct_block(conditional.then).0.statements);

        // Remove condition from the condition stack.
        self.condition_stack.pop();

        // Consume the otherwise-block and flatten its constituent statements into the current block.
        if let Some(statement) = conditional.otherwise {
            // Add the negated condition to the condition stack.
            self.condition_stack.push(Expression::Unary(UnaryExpression {
                op: UnaryOperation::Not,
                receiver: Box::new(conditional.condition.clone()),
                span: conditional.condition.span(),
            }));

            // Reconstruct the otherwise-block and accumulate it constituent statements.
            match *statement {
                Statement::Block(block) => statements.extend(self.reconstruct_block(block).0.statements),
                _ => unreachable!("SSA guarantees that the `otherwise` is always a `Block`"),
            }

            // Remove the negated condition from the condition stack.
            self.condition_stack.pop();
        };

        (Statement::dummy(Default::default()), statements)
    }

    /// Rewrites a console statement into a flattened form.
    /// Console statements at the top level only have their arguments flattened.
    /// Console statements inside a conditional statement are flattened to such that the check is conditional on
    /// the execution path being valid.
    /// For example, the following snippet:
    /// ```leo
    /// if condition1 {
    ///    if condition2 {
    ///        console.assert(foo);
    ///    }
    /// }
    /// ```
    /// is flattened to:
    /// ```leo
    /// console.assert(!(condition1 && condition2) || foo);
    /// ```
    /// which is equivalent to the logical formula `(condition1 /\ condition2) ==> foo`.
    fn reconstruct_console(&mut self, input: ConsoleStatement) -> (Statement, Self::AdditionalOutput) {
        let mut statements = Vec::new();

        // Flatten the arguments of the console statement.
        let console = ConsoleStatement {
            span: input.span,
            function: match input.function {
                ConsoleFunction::Assert(expression) => {
                    let (expression, additional_statements) = self.reconstruct_expression(expression);
                    statements.extend(additional_statements);
                    ConsoleFunction::Assert(expression)
                }
                ConsoleFunction::AssertEq(left, right) => {
                    let (left, additional_statements) = self.reconstruct_expression(left);
                    statements.extend(additional_statements);
                    let (right, additional_statements) = self.reconstruct_expression(right);
                    statements.extend(additional_statements);
                    ConsoleFunction::AssertEq(left, right)
                }
                ConsoleFunction::AssertNeq(left, right) => {
                    let (left, additional_statements) = self.reconstruct_expression(left);
                    statements.extend(additional_statements);
                    let (right, additional_statements) = self.reconstruct_expression(right);
                    statements.extend(additional_statements);
                    ConsoleFunction::AssertNeq(left, right)
                }
            },
        };

        // Add the appropriate guards.
        match self.construct_guard() {
            // If the condition stack is empty, we can return the flattened console statement.
            None => (Statement::Console(console), statements),
            // Otherwise, we need to join the guard with the expression in the flattened console statement.
            // Note given the guard and the expression, we construct the logical formula `guard => expression`,
            // which is equivalent to `!guard || expression`.
            Some(guard) => (
                Statement::Console(ConsoleStatement {
                    span: input.span,
                    function: ConsoleFunction::Assert(Expression::Binary(BinaryExpression {
                        // Take the logical negation of the guard.
                        left: Box::new(Expression::Unary(UnaryExpression {
                            op: UnaryOperation::Not,
                            receiver: Box::new(guard),
                            span: Default::default(),
                        })),
                        op: BinaryOperation::Or,
                        span: Default::default(),
                        right: Box::new(match console.function {
                            // If the console statement is an `assert`, use the expression as is.
                            ConsoleFunction::Assert(expression) => expression,
                            // If the console statement is an `assert_eq`, construct a new equality expression.
                            ConsoleFunction::AssertEq(left, right) => Expression::Binary(BinaryExpression {
                                left: Box::new(left),
                                op: BinaryOperation::Eq,
                                right: Box::new(right),
                                span: Default::default(),
                            }),
                            // If the console statement is an `assert_ne`, construct a new inequality expression.
                            ConsoleFunction::AssertNeq(left, right) => Expression::Binary(BinaryExpression {
                                left: Box::new(left),
                                op: BinaryOperation::Neq,
                                right: Box::new(right),
                                span: Default::default(),
                            }),
                        }),
                    })),
                }),
                statements,
            ),
        }
    }

    /// Static single assignment converts definition statements into assignment statements.
    fn reconstruct_definition(&mut self, _definition: DefinitionStatement) -> (Statement, Self::AdditionalOutput) {
        unreachable!("`DefinitionStatement`s should not exist in the AST at this phase of compilation.")
    }

    /// Replaces a finalize statement with an empty block statement.
    /// Stores the arguments to the finalize statement, which are later folded into a single finalize statement at the end of the function.
    fn reconstruct_finalize(&mut self, input: FinalizeStatement) -> (Statement, Self::AdditionalOutput) {
        // Construct the associated guard.
        let guard = self.construct_guard();

        // For each finalize argument, add it and its associated guard to the appropriate list of finalize arguments.
        // Note that type checking guarantees that the number of arguments in a finalize statement is equal to the number of arguments in to the finalize block.
        for (i, argument) in input.arguments.into_iter().enumerate() {
            // Note that the argument is not reconstructed.
            // Note that this unwrap is safe since we initialize `self.finalizes` with a number of vectors equal to the number of finalize arguments.
            self.finalizes.get_mut(i).unwrap().push((guard.clone(), argument));
        }

        (Statement::dummy(Default::default()), Default::default())
    }

    // TODO: Error message requesting the user to enable loop-unrolling.
    fn reconstruct_iteration(&mut self, _input: IterationStatement) -> (Statement, Self::AdditionalOutput) {
        unreachable!("`IterationStatement`s should not be in the AST at this phase of compilation.");
    }

    /// Transforms a return statement into an empty block statement.
    /// Stores the arguments to the return statement, which are later folded into a single return statement at the end of the function.
    fn reconstruct_return(&mut self, input: ReturnStatement) -> (Statement, Self::AdditionalOutput) {
        // Construct the associated guard.
        let guard = self.construct_guard();

        // Add it to `self.returns`.
        // Note that SSA guarantees that `input.expression` is either a literal or identifier.
        match input.expression {
            // If the input is an identifier that maps to a tuple, add the corresponding tuple to `self.returns`
            Expression::Identifier(identifier) if self.tuples.contains_key(&identifier.name) => {
                // Note that the `unwrap` is safe since the match arm checks that the entry exists in `self.tuples`.
                let tuple = self.tuples.get(&identifier.name).unwrap().clone();
                self.returns.push((guard, Expression::Tuple(tuple)))
            }
            // Otherwise, add the expression directly.
            _ => self.returns.push((guard, input.expression)),
        };

        (Statement::dummy(Default::default()), Default::default())
    }
}
