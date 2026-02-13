
import os
import threading
from pybox.exception import PyboxException
from pybox.box import PyBox

def new_pybox(preopen_dirs={}):
    box = PyBox(preopen_dirs)
    box.init_local('1')
    return ('1',box)


def test_protect():
    id,box = new_pybox()
    box.protect(id,"protected")
    output = box.exec("protected = 10",id)
    assert "Cannot modify protected" in output

def test_inherit():
    id,box = new_pybox()
    code = """
root = "I'am root"
    """
    box.exec(code,id)
    box.init_local_from("child",id)
    code = """
print(root)
root = None
"""
    assert "I'am root" in box.exec(code,"child")
    code = """
print(root)
"""
    assert "I'am root" in box.exec(code,id)


# def test_handler():
#     pass


def test_tool():
    id,box = new_pybox()
    @box.tool
    def hello(name):
        return f'Hello {name}'
    
    box.exec(hello.stub(),id)

    code = """
print(hello('pybox'))
    """

    assert "Hello pybox" in box.exec(code,id)


def test_reentrant():
        id,box = new_pybox()
        @box.tool
        def hello(name):
            nonlocal id,box
            return box.exec(f"print('Hello {name}')",id)
        
        box.exec(hello.stub(),id)

        code = """
print(hello('pybox'))
        """

        assert "Hello pybox" in box.exec(code,id)


def test_consistency():
    caller_thread = threading.current_thread()
    id,box = new_pybox()
    @box.tool
    def test():
        assert threading.current_thread() == caller_thread
        return
    
    box.exec(test.stub(),id)

    code = """
print(test())
    """

    assert "None" in box.exec(code,id)


def test_directory():
    id,box = new_pybox(
        {
            "/":os.path.dirname(__file__)
        }
    )

    code = """
import os
print(os.listdir('/'))
    """

    assert "test_pybox.py" in box.exec(code,id)




    


def test_exception():
    id,box = new_pybox()
    exception = PyboxException("Test Exception")
    @box.tool
    def test_exception():
        nonlocal exception
        raise exception
    
    box.exec(test_exception.stub(),id)

    code = """
print(test_exception())
    """

    try:
        print(box.exec(code,id))
    except Exception as e:
        assert e==exception
        

def test_thread():
    import threading
    import time
    id,box = new_pybox()
    @box.tool
    def sleep():
        time.sleep(1)
        return "Test Thread"
    
    box.exec(sleep.stub(),id)

    code = """
print(sleep())
    """

    def run():
        nonlocal code
        if not "Test Thread" in box.exec(code,id):
            raise BaseException("Thread Test Failed!")

    run_thread = threading.Thread(
        target=run,
        daemon=True
    )

    run_thread.start()
    try:
        time.sleep(0.1)
        box.exec(code,id)
        raise BaseException("Thread Test Failed!")
    except:
        pass
    run_thread.join()

if __name__ == '__main__':
    test_protect()
    test_inherit()
    test_tool()
    test_reentrant()
    test_exception()
    test_consistency()
    test_directory()
    test_thread()