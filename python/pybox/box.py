
import os
import json
from typing import Callable, Dict, Any

from .exception import PyboxException
from .pyboxcore import PyBoxReactor
from .tool import PyboxPTCTool


class PyBoxHandler:

    def __init__(self, handle: int, func: Callable[[bytes], bytes]):
        self.handle = handle
        self.func = func

    def __call__(self, data:bytes):
        return self.func(data)


class PyBoxJSONRPCHandler(PyBoxHandler):

    def __init__(self, handle: int, callback: Callable[..., Any]):
        self.callback = callback
        super().__init__(handle, self._handler_impl)

    def _handler_impl(self, data: bytes) -> bytes:
        try:
            request = json.loads(data.decode('utf-8'))
            args = request.get("args", [])
            kwargs = request.get("kwargs", {})
            result = self.callback(*args, **kwargs)
            response_data = json.dumps({"result": result}).encode('utf-8')
            return response_data
        except PyboxException:
            # use this Exception to escape from sandbox
            raise
        except Exception as e:
            error_response = {
                "exception": f"{type(e).__name__}: {str(e)}",
                # "traceback": traceback.format_exc()
            }
            return json.dumps(error_response).encode('utf-8')
        
        # # wasmtime cannot handle BaseException，we need wrap it
        # except BaseException as e:
        #     # this will triggle the wasm runtime execution stopped
        #     raise PyboxException("WASM internal exeception",e) from e




class PyBox(PyBoxReactor):
    """
    Python wrapper for PyBoxReactor with automatic WASM file loading
    """

    def __init__(self, preopen_dirs={}):
        """
        Initialize PyBox with optional preopen directories and environment variables

        Args:
            preopen_dirs: Dictionary mapping guest paths to host paths (GUEST:HOST)
            env_vars: Dictionary of environment variables (currently not used)
        """
        image_dir = os.path.join(os.path.dirname(__file__), "image")
        # Find the WASM file
        wasm_file = os.path.join(image_dir, "pybox_reactor.wasm")

        # Call parent __init__ to initialize the reactor
        super().__init__(wasm_file, preopen_dirs)

        self._handlers: Dict[int, PyBoxHandler] = {}


    def tool(self,func:Callable):
        handler = PyBoxJSONRPCHandler(
                len(self._handlers),
                func
            )
        self.register_handler(handler.handle,handler)
        return PyboxPTCTool(
            handler.handle,
            func
        )


__all__ = [
    PyBoxHandler.__name__,
    PyBoxJSONRPCHandler.__name__,
    PyBox.__name__
]