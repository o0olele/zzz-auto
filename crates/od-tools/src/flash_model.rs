use anyhow::{Context, Result, ensure};

// 只处理本项目固定分类模型的 ONNX protobuf，未知字段原样保留。
// 不解析权重，也不依赖 Python 或修改来源文件。
struct Field<'a> {
    number: u64,
    wire: u64,
    raw: &'a [u8],
    value: &'a [u8],
}

fn varint(bytes: &[u8], cursor: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let byte = *bytes.get(*cursor).context("ONNX varint 被截断")?;
        *cursor += 1;
        ensure!(shift != 63 || byte <= 1, "ONNX varint 溢出");
        value |= u64::from(byte & 127) << shift;
        if byte < 128 {
            return Ok(value);
        }
    }
    anyhow::bail!("ONNX varint 无效")
}

fn fields(bytes: &[u8]) -> Result<Vec<Field<'_>>> {
    let mut cursor = 0;
    let mut result = Vec::new();
    while cursor < bytes.len() {
        let start = cursor;
        let tag = varint(bytes, &mut cursor)?;
        let number = tag >> 3;
        let wire = tag & 7;
        ensure!(number > 0 && number < (1 << 29), "ONNX 字段编号无效");
        let length = match wire {
            0 => {
                let value_start = cursor;
                varint(bytes, &mut cursor)?;
                cursor - value_start
            }
            1 => 8,
            2 => usize::try_from(varint(bytes, &mut cursor)?)?,
            5 => 4,
            _ => anyhow::bail!("不支持的 ONNX wire type {wire}"),
        };
        let value_start = if wire == 0 { cursor - length } else { cursor };
        if wire != 0 {
            cursor = cursor.checked_add(length).context("ONNX 字段长度溢出")?;
        }
        ensure!(cursor <= bytes.len(), "ONNX 字段被截断");
        result.push(Field {
            number,
            wire,
            raw: &bytes[start..cursor],
            value: &bytes[value_start..cursor],
        });
    }
    Ok(result)
}

fn one<'a>(fields: &'a [Field<'_>], number: u64, wire: u64) -> Result<&'a [u8]> {
    let mut found = fields.iter().filter(|field| field.number == number);
    let field = found.next().context("ONNX 缺少必需字段")?;
    ensure!(
        field.wire == wire && found.next().is_none(),
        "ONNX 字段类型或数量无效"
    );
    Ok(field.value)
}

fn integer(bytes: &[u8]) -> Result<u64> {
    let mut cursor = 0;
    let value = varint(bytes, &mut cursor)?;
    ensure!(cursor == bytes.len(), "ONNX 整数字段无效");
    Ok(value)
}

fn shape(info: &[u8]) -> Result<Vec<u64>> {
    let info = fields(info)?;
    let ty = fields(one(&info, 2, 2)?)?;
    let tensor = fields(one(&ty, 1, 2)?)?;
    ensure!(integer(one(&tensor, 1, 0)?)? == 1, "分类模型须使用 float32");
    let dimensions = fields(one(&tensor, 2, 2)?)?;
    dimensions
        .iter()
        .map(|dimension| {
            ensure!(
                dimension.number == 1 && dimension.wire == 2,
                "ONNX 维度无效"
            );
            let dimension = fields(dimension.value)?;
            ensure!(dimension.len() == 1, "分类模型维度须固定");
            integer(one(&dimension, 1, 0)?)
        })
        .collect()
}

fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 128 {
        output.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    output.push(value as u8);
}

fn encode_bytes(number: u64, value: &[u8], output: &mut Vec<u8>) {
    encode_varint((number << 3) | 2, output);
    encode_varint(value.len() as u64, output);
    output.extend_from_slice(value);
}

pub fn remove_output_softmax(model: &[u8]) -> Result<Vec<u8>> {
    ensure!(model.len() <= 16 * 1024 * 1024, "闪光模型超出转换大小上限");
    let model_fields = fields(model)?;
    let graph_bytes = one(&model_fields, 7, 2)?;
    let graph = fields(graph_bytes)?;
    let input = one(&graph, 11, 2)?;
    let output = one(&graph, 12, 2)?;
    ensure!(
        shape(input)? == [1, 3, 640, 640],
        "闪光模型输入须为 [1,3,640,640]"
    );
    ensure!(shape(output)? == [1, 3], "闪光模型输出须为 [1,3]");
    let output_info = fields(output)?;
    let output_name = one(&output_info, 1, 2)?;
    let mut producers = Vec::new();
    for (index, field) in graph
        .iter()
        .enumerate()
        .filter(|(_, field)| field.number == 1)
    {
        ensure!(field.wire == 2, "ONNX 节点类型无效");
        let node = fields(field.value)?;
        if node
            .iter()
            .any(|field| field.number == 2 && field.value == output_name)
        {
            producers.push((index, node));
        }
    }
    ensure!(producers.len() == 1, "闪光输出须有唯一生成节点");
    let (producer_index, node) = producers.pop().unwrap();
    ensure!(one(&node, 4, 2)? == b"Softmax", "闪光输出节点不是 Softmax");
    ensure!(!one(&node, 1, 2)?.is_empty(), "Softmax 输入为空");
    ensure!(one(&node, 2, 2)? == output_name, "Softmax 输出无效");
    ensure!(
        node.iter()
            .all(|field| field.number != 7 || field.value.is_empty()),
        "Softmax 必须使用 ONNX 默认域"
    );
    let attributes = one(&node, 5, 2)?;
    let axis = fields(attributes)?;
    ensure!(
        one(&axis, 1, 2)? == b"axis" && integer(one(&axis, 3, 0)?)? == 1,
        "Softmax 须为 axis=1"
    );
    ensure!(
        integer(one(&axis, 20, 0)?)? == 2,
        "Softmax axis 须为整数属性"
    );
    let mut replacement = Vec::new();
    for field in &node {
        match field.number {
            4 => encode_bytes(4, b"Identity", &mut replacement),
            5 => {}
            _ => replacement.extend_from_slice(field.raw),
        }
    }
    let mut new_graph = Vec::with_capacity(graph_bytes.len());
    for (index, field) in graph.iter().enumerate() {
        if index == producer_index {
            encode_bytes(1, &replacement, &mut new_graph);
        } else {
            new_graph.extend_from_slice(field.raw);
        }
    }
    let mut derived = Vec::with_capacity(model.len());
    for field in &model_fields {
        if field.number == 7 {
            encode_bytes(7, &new_graph, &mut derived);
        } else {
            derived.extend_from_slice(field.raw);
        }
    }
    Ok(derived)
}
