# 0005 用 Hello World 学 Rust 最基础语法

## 问题

我对 Rust 完全不熟，想先从一个 Hello World 学基础语法。

## 简短结论

Rust 入门可以先只看这几个东西：

- `fn main()`
- `println!`
- `let`
- `mut`
- 基本字符串插值

掌握这几个之后，再看结构体、枚举、trait 就不会那么突然。

## 示例代码

文件位置：

- `playground/rust-hello/hello.rs`

代码如下：

```rust
fn main() {
    println!("Hello, Pony Agent!");

    let language = "Rust";
    let mut version = 1;

    println!("We are learning {} basics.", language);

    version += 1;
    println!("Practice round: {}", version);
}
```

## 系统化梳理

### 1. `fn main()`

这是 Rust 程序入口。

- `fn` 表示定义函数
- `main` 是程序启动时默认执行的函数名

可以先把它理解成：

- JavaScript 里的程序入口
- Python 里脚本最开始执行的主流程

### 2. `println!`

这是标准输出宏，用来打印内容。

注意它后面有 `!`，因为它是宏，不是普通函数。

例如：

```rust
println!("Hello");
println!("Value: {}", 3);
```

### 3. `let`

Rust 用 `let` 定义变量：

```rust
let language = "Rust";
```

默认情况下，Rust 变量是不可变的。

### 4. `mut`

如果变量后面还要改，就要加 `mut`：

```rust
let mut version = 1;
version += 1;
```

这表示“这个变量是可变的”。

### 5. 语句结尾的分号

Rust 大多数普通语句后面都要加分号：

```rust
let x = 1;
println!("{}", x);
```

### 6. 字符串占位符

Rust 打印变量时常用 `{}`：

```rust
println!("Language: {}", language);
```

这和 Python 的 f-string 不完全一样，更像格式化占位输出。

## 如何运行

在项目根目录执行：

```powershell
rustc playground/rust-hello/hello.rs -o playground/rust-hello/hello.exe
./playground/rust-hello/hello.exe
```

## 你现在最该先建立的感觉

先不要急着记复杂语法，先有这几个基本印象：

- Rust 很重视“明确”
- 变量默认不可变
- 输出和格式化比较规整
- 语法看起来有点像 C 系语言，但约束更强

## 常见误区

- 误区 1：`println!` 是普通函数
- 误区 2：Rust 变量默认都能改
- 误区 3：一开始就该学所有权、生命周期

## 后续值得继续学什么

- 基本数据类型
- `if` / `match`
- 函数参数和返回值
- `struct` 和 `enum`

## 可延展内容选题

- 公众号：`完全不懂 Rust，怎么从一个 Hello World 开始入门`
- 知乎：`Rust 新手第一天到底该先学什么？`
