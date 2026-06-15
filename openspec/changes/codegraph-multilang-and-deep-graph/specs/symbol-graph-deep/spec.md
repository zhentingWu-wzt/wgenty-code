## ADDED Requirements

### Requirement: Inherits relationship

系统 SHALL 支持继承/实现关系。

#### Scenario: Java extends
- **WHEN** 解析 `class Dog extends Animal`
- **THEN** 产生 RelKind::Inherits 关系 (Dog → Animal)

#### Scenario: Rust impl
- **WHEN** 解析 `impl Display for Foo`
- **THEN** 产生 RelKind::Inherits 关系 (Foo → Display)

### Requirement: TypeOf relationship

系统 SHALL 支持变量类型归属关系。

#### Scenario: Variable type
- **WHEN** 解析 `let x: String` 或 `String name`
- **THEN** 产生 RelKind::TypeOf 关系 (x/name → String)

### Requirement: Returns relationship

系统 SHALL 支持函数返回值类型关系。

#### Scenario: Return type
- **WHEN** 解析 `fn foo() -> Bar`
- **THEN** 产生 RelKind::Returns 关系 (foo → Bar)

### Requirement: Parameter relationship

系统 SHALL 支持函数参数类型关系。

#### Scenario: Parameter type
- **WHEN** 解析 `fn foo(x: i32, y: &str)`
- **THEN** 产生 RelKind::Parameter 关系 (foo → i32, foo → &str)
