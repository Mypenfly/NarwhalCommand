# N_Edit

本项目是由设计Ncoding项目的文件操作命令中edit操作的延伸设计。

Ncoding 项目地址：https://github.com/Mypenfly/Ncoding.git

有关开发文档：./ncoding.md

项目特点：

1. 语义级设计命令语法，为的是降低LLM/人类的使用、学习成本。
2. 逐行、逐个字符操作，确保精准。
3. 报错设置，为的是让模型/人类快速理解错误，并给予适当提示。
4. 基于rust确保类型和内存安全。

## 语法设计

首先，介绍一下本项目设计的语义级语法。
当条命令由两个部分组成，一个部分是用于区别现有的一些语言已有的标识符，另一部分则是语义设计的命令语句。

标识符:

1. `//!@` 这是唯一确定的标识符，用于程序执行命令的识别，希望有效避免和其它既有语法冲突，同时学习成本较低（类似注释）
2. `...` 省略符号，是伪代码的常用符号，可以用于省略，也是执行匹配的一个判断符号。

命令语句头：

1. `Open:` 打开/读取一个文件的内容。
2. `Location:` 用于定位语句/字符，是检索/修改的首要。
3. `Delete: ` 删除指定的行。
4. `New:` 新增指定内容。
5. `Off: ` 用于终止/标识一个执行块的终止。(后面详细讲解) 

> **注意**：命令语句头执行时识别并不要求格式，可以全大写如：OPEN,也可以全小写：open,识别时优先处理为全大写，然后再识别命令。同时也非空格敏感，也就是也要除去空格后识别。

下面讲解具体命令语句执行机制和有关示例

### Open:

本命令用于打开并读取文件内容，需要一个文件地址参数，使用示例：
```text
  //!@Open: ./example.rs
```
执行流：

1. 识别到Open命令，提取出文件路径。缺少文件路径，抛出错误： “Open命令缺失一个必要的文件路径”
2. 得到文件路径，优先判断是否存在，不存在抛出错误： “Open命令的给定路径： ... 不存在”
3. 文件存在尝试打开文件，提取其中内容。失败抛出错误： “Open命令给定路径： ... 不可打开，原因： ...”
4. 将提取出的内容解析为数据结构`FileContent`。

> 此处相匹配的是`//!@Off: Open` 这个命令表示关闭/推出上一级Open命令，也即将经修改的FileContent的内容写回原文件。
同样的如果，执行结束时没有遇到Off命令，则默认为Off:Open,执行对应操作

### Location:

该命令是匹配，精准修改的关键，核心机制是**保留格式，忽略空格差异的字符级匹配**，可以接受一个Block子命令使用示例：
```rust
  // 这是无Block命令，是按给定的行定位一个code block
  //!@Location:
  fn example() -> Option<()> {
    let x = 0 ;
    let y = "Hello";
    ...
```
```rust
  // 这是考虑一个Block的定位
  //!@Location:Block
  fn block_example() -> usize {
```

先讲解`Location`的工作原理，再说明有无block的差异。
执行流：

1. 识别到Location命令，判断是否有block指令，走对应分支。如果有未知的指令则抛出错误： “Block命令接受了一个错误的指令：...” 。
2. 提取Location命令后的从第一行非`//!@`开头的行到`...`（分隔符）或者下一个`//!@`行的所有内容。（**注意**：是从Location后第一行非`//!@`开头开始提取，也即是说允许两行连续的`//!@`开头的命令语句）
3. 解析提取内容为数据结构 `LocationContent`，对应的结构如下：
```rust
  struct LocationContent {
    lines:Vec<LocationLine> // 对每一行都要额外解析
  }

  struct LocationLine {
    index: usize, // 从0开始的原始序号
    diff_taps: Option<usize>, // 记录差异缩进数量（空格数量），计算方式是index = 0（第一行）定义为0,然后后面的缩进数量减去第一行到原始缩进数量
    content: String, //这一行的内容，未经过处理，保留了缩进和其中的空格。
    line_num: Option<usize> // 定位对应于原文的行号，未解析时是None
  }
```
4. 解析为对应结构后，提取第一行（index =0）内容，按无缩进，无空格进行纯字符匹配原文内容，将提取的匹配的那一行以及后面的与location content同样行数的内容一起解析为结构 `FirstMatchContents`，结构如下：
```rust
  struct FirstMatchContents {
    contents:Vec<FirstMatchContent> // 将所有首行匹配的内容收集为一个vec
  }

  struct FirstMatchContent {
    start_line : usize , // 匹配到的那一行的行号
    lines : Vec<MatchLine>,// 逐行解析
  }

  struct MatchLine {
    line_num : usize, // 每一行的对应行号
    taps: usize, // 对应行的原始缩进数量
    diff_taps: usize, // 缩进差异数量，计算方式和前面一致
    content: String , // 对应行的内容，未处理
  }
```
5. 解析来对每个FirstMatchContent和LocationContent的内容进行逐行（跳过空行）匹配，
匹配要求首先是对应的content的无缩进/无空格的纯字符匹配，其次是diff_taps的匹配，每一行匹配结束后都删除不符合要求的FirstMatchContent数据，这样就完成了保留格式的纯字符匹配。
这一步匹配结果应该只有一个FirstMatchContent满足要求，如果不是，那么则抛出异常：
```text
  Location命令匹配得到过多结果/没有结果，请检查(建议增加location指定内容/更改location位置)：
  {{LocationContent}} // 这里是写index+1  content格式的LocationContent内容(content为原格式)
  目前检索得到的有：
  {{FirstMatchContents}} // 这里填入检索结束后保留的内容格式line_num content 。注意这里最多输出3个FirstMatchContent,如果超过3个则在最后一行输出(n more)
```
6. 对于匹配后得到的唯一FirstMatchContent ，按第一行的taps,在原文数据中向下找到第一个同taps数量的行（跳过空行），提取为一个`ContentBlock`。（这里是运用当前几乎所有的语言的规范都能或多或少能按照缩进划分层级的特点）
数据结构示例如下：
```rust
  struct ContentBlock {
    start_line : usize , // 首行的对应行号，应该和FirstMatchContent中的start_line一致
    lines : Vec<Line> // 对每一行进行解析，其实可以复用MatchLine
  }

  struct Line {
    line_num : usize,
    taps: usize,
    diff_taps: usize,
    content : String
  }
```
> 注意一个Block的识别标准不应该只是taps数量来判定，可以考虑识别location/math得到的首行的末尾是不是`{`然后去找对应的`}`（要考虑多层级嵌套），而对于python/yaml这类缩进严格语法而言缩进依然是最首要的。
同时，检索/解析block也不是一定会生效的，因为location内容可能是一个block/嵌套的内部的内容（可以根据FirstMatchContent中lines中的diff_taps是否都为0可以判断）。
对于没有锁定block的Location内容或者无法提取block的内容的文件（如markdown文件）我们此时的解析为ContentBlock的对象为从location内容第一行后的所有内容(这个状态是Block不可解析)，同时拒绝Block指令，也即不接受`Location:Block`,此时报错为
： “Location被指定为一个Block,但提供内容无法解析为一个Block,请重新定位：{{LocationContent}}”

**注意**： 对于没有Block指令,后面的所有操作，无论Delete,New或者又一次Location都是发生在上一步解析得到的ContentBlock内部，而原始文件中的其他内容不受影响，知道程序执行遇到命令语句`Off:Location`,
此时，Off命令意味着退出上一个Location命令，并将对应的修改后的ContentBlock内容按行号格式写回原文件内容，实现对应修改。
而针对有Block指令，且block可以解析，则修改发生在对应的block后，且修改首行缩进格式同block首行，这经常发生在新增一个方法/整个删除一个方法时使用，例如：
```text
  // 这里location一整个example方法，也即ContentBlock中存有的就是逐行解析后的example方法
  //!@Location:Block
  fn example() -> Option<()> {
  //!@New:
  fn new() -> Self {
    Self{
      a : 0
    } // 这里就在examle方法后新增了一个新的new方法
  }
```
对应的原文件内容可能是：
```rust
  fn example() -> Option<()> {
    ...
  }
  fn delete() {
    ...
  }
```
修改后文件为 :
```rust
  fn example() -> Option<()> {
    ...
  }
  fn new() -> Self {
    ...
  }
  fn delete() {
    ...
  }
```

另外，**对于Location的嵌套使用**，其匹配机制和前面所述一致，只不过匹配发生范围从全文内容变成上一个ContentBlock中的内容。（这意味这其实Open得到的`FileContent`的数据结构和`ContentBlock`的数据结构一致/相似），也同样会得到一个更小范围的ContentBlock(从一个方法可能变成一个for循环内/一个条件判断的分支内)

### New:

本命令的操作是针对上一个Location解析得到的ContentBlock中的内容，所以必须配合Location使用，也即必须在New前面找到`//!@Location:`，而且不能在这之前出现`...`(因为分隔符，也是省略号，如果New前是`...`就会导致new的内容不知道插入何处)
。本命令本质上是一个按格式插入，使用示例：
```text
  //!@Location:
  fn example() -> Option<()> {
    ...
    //!@Location:
    for i in big_list {
      let x = get_x() ;
      //!@New:
      let m = x * i
      if m> 20 :{
        return None
      }
      ...
```

执行流：

1. 程序识别到`//!@New:`，立即检查前文中是否有Location命令，也即是否在一个确定的ContentBlock内部，如果不是，抛出错误： “New命令发生在一个不确定的位置，请在此之前指定Location”
2. 提取并解析New的内容为`NewContent`（从这一个New开始到下一个命令语句或者分隔符出现为止）,结构示例如下：
```rust
  struct NewContent {
    lines:Vec<NewLine>
  }
  struct NewLine {
    diff_taps: usize,// 计算方式和前文一致
    content: String // 去除缩进的内容（但保留其中的空格）
  }
```
3. 查找New前一行/location内容的最后一行在ContentBlock中的位置，然后，将New的内容插入进去。
插入要求，首先找到location最后一行在ContentBlock中的下一行，获取它的taps和diff_taps,然后按照NewLine的diff_taps，同格式逐行插入到location最后一行的下面
4. 插入结束，对ContentBlock进行一次检查（按照loction匹配思路），检查插入的格式和内容无误。
5. 一切结束后输出一段内容，格式为：
```text
  {{ContentBlock}} // 修改后的ContentBlock 格式为line_num content,其中新增行前面加一个绿色的"+"
```

> 此时如果遇到//!@Off:New,这个的效力等同于... 无特殊操作

> 对于第一点的检查有两个意外情况，也即提供了两个特殊指令 `//!@New:Start` `//!@New:End` 这时不要求前一个命令语句是Location,而是Open/Off语句，Start意味直接在文件开头加入内容，End意味着直接在文件末尾加入内容，此时新增的内容都首行的缩进为0,后面的缩进根据diff_taps得到。

### Delete:

和New一样，发生在上一个Location解析得到的ContentBlock中，但如果和Location:Block一起使用可以支持Block指令，
一般使用示例：
```text
  
  //!@Location:
  fn example() -> Option<()> {
    ...
    //!@Location:
    for i in big_list {
      let x = get_x() ;
      //!@Delete:
      let m = x * i
      if m> 20 :{
        return None
      }
      ...
```

执行流：

1. 识别到Delete命令，同New检查是否有Location,如无，抛出错误。
2. 按嵌套Location的逻辑在ContentBlock中匹配Delete内容，并得到对应行号（要连续，不能跳行，也不支持用 ... 省略，此时的 `...` 是分隔符，遇到了就是Delete内容提取末尾）。
如果匹配失败，抛出错误，格式为：
```text
  Delete命令错误，没有找到：
  ...  // Delete内容
  当前的Block为：
  ... // ContentBlock内容
```
3. 按对应行号删除ContentBlock中的内容，之后重新对ContentBlock中的剩余内容重新分配行号（要和原文对应，其实也即是根据首行的line_num分配）
4. 检查删除是否成功，是否误删除内容（按location逻辑）
5. 一切完成，输出内容格式为：
```text
  {{ContentBlock}} // 删除前的ContentBlock ，其中匹配到的删除内容前用 红色 "-" 标注
```

而对于指定了Block指令的Delete而言，也即 `//!@Delete:Block`，
需要判断前一个Location使用也Block指令（这是确保location的block可解析,而不是一整个后半文件内容），然后对整个ContentBlock中的内容删除，仅保留首行的行号（为了填充回/替换原文数据做准备，避免修改后原文中出现大量空行）
使用示例:
```text
  //!@Location:
  fn example() -> Option<()> {
    ...
    // 这里使用了两个指令堆叠使用的形式，这是因为Location不会提取紧接着的命令语句作为匹配依据（但注意“//”注释内容也可以融入匹配）
    //!@Location:
    //!@Delete:Block
    for i in big_list {
      let x = get_x() ;
      let m = x * i
      if m> 20 :{
        return None
      }
      ...
```

现在有了Delte和New我们就可以实现精准的文件编辑操作，不过Delete/New的配合有两点需要格外注意就是：

1. 不要造成过多的空行
2. 修改位置/格式不要出错。

## 预期产品

程序(二进制)执行：
```sh
nedit example.txt
```
或者，定义一个文件后缀（像.nedit .nd .ned之类）：
```sh
nedit example.nedit
```
程序执行是按脚本形式，逐行读取内容执行的。

预期输出有良好的报错处理（我已给出一个初步的报错），以及一个有高亮的好输出形式（尤其是New和Delete的输出）

## 一些发散的想法/扩展方向

1. 支持一个Include命令，用于将多个修改脚本拼接成一个，避免一个脚本过大过长，不易管理。
2. 支持一个Async命令和Off:Async配合，将不同的块（非嵌套Location）进行异步并行处理，支持批量，高效处理多个文件/单一大文件
3. Location 支持使用 `//!@Location:@66,120`这样的按行号指定来得到Location内容，以及进一步得到对应的ContentBlock ✅ 已实现
4. 实现修改可逆转，通过同一个修改用脚本。（这里的问题在于使用Delete:Block命令操作之后，好像无法逆转？）
