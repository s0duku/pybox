
import inspect

class PyboxPTCTool:
    def __init__(
        self,
        handle,
        tool_func
    ):
        self._handle = handle
        self._function = tool_func

    @property
    def handle(self):
        return self._handle

    @property
    def name(self):
        return self._function.__name__

    @property
    def function(self):
        return self._function

    @staticmethod
    def _format_docstring(doc):
        """format docstring"""
        if not doc:
            return ''

        # 如果是单行文档，用单行格式；如果是多行，用多行格式
        if '\n' in doc:
            # 多行文档字符串：每行缩进，使用标准的多行格式
            import textwrap
            indented_doc = textwrap.indent(doc, '    ')
            return f'\n    """\n{indented_doc}\n    """'
        else:
            # 单行文档字符串
            return f'\n    """{doc}"""'


    def stub(self):
        func_name = self.name
        sig = inspect.signature(self._function)
        doc = inspect.getdoc(self._function)

        # 提取所有参数名，用于填充 pybox_json_rpc 调用
        param_names = list(sig.parameters.keys())
        params_call = ', '.join(param_names)

        stub_str = f"def {func_name}{sig}:"
        stub_str += PyboxPTCTool._format_docstring(doc)
        stub_str += f'\n    return pybox_json_rpc({self._handle}, {params_call})'

        return stub_str


    def decl(self):
        """生成函数声明（包含签名和文档字符串，不包含函数体）"""
        func_name = self.name
        sig = inspect.signature(self._function)
        doc = inspect.getdoc(self._function)

        decl_str = f"def {func_name}{sig}:"
        decl_str += self._format_docstring(doc)

        return decl_str

    def __call__(self, *args, **kwargs):
        return self._function(
            *args,
            **kwargs
        )
    

__all__ = [
    PyboxPTCTool.__name__
]